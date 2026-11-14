//! 右パネル（ターミナルが 0 枚のときはメイン領域）に出す、1 つのファイルの表示と編集（設計書 §5.3）。
//!
//! 全体の表示は GPUI Kit の `Editor`、差分の表示は `Diff`（Unified、閲覧専用）を使う。読み書きはすべて
//! ProjectFs を通し、裏のスレッドで行う（WSL のプロジェクトは fude serve と話すため）。

use std::io;
use std::sync::Arc;

use fude_core::claude::selected_lines;
use fude_core::markdown::{is_markdown, split_preview};
use fude_core::panel::{ExternalAction, FileMode, ReadOutcome, external_change};
use fude_core::text::{OpenError, TextDoc, decode_text, encode_text};
use fude_fs::git::{GitMark, LineMark, line_marks};
use fude_fs::{ProjectFs, RelPath};
use fude_core::workspace::ProjectRoot;
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{EditorState, GutterDecoration, GutterDecorationCollection, InputEvent, Position};
use gpui_kit::component::diff::{Diff, DiffFile, DiffState};
use gpui_kit::component::input::Editor;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::dialog::error_text;
use crate::language::language_for;
use crate::md_preview::MdPreview;
use crate::theme::{Theme, mono_font, popup_shadow, size, ui_font};
use crate::title_row::icon;

gpui_kit::actions!(file_view, [SaveFile, LeaveEditor, SendToClaude, ToggleMarkdownMode]);

/// Esc でターミナルへ戻る（FileView の文脈）。エディタ列は閉じない。
pub fn bind_file_view_keys(cx: &mut App) {
    // 保存は fude_core::keymap::context_bindings（workspace_view::bind_keys）で登録する。
    cx.bind_keys([KeyBinding::new("escape", LeaveEditor, Some("FileView"))]);
    // gpui-kit は macOS の置換を ⌘⇧F に割り当てているが、⌘⇧F は全体検索に使う（設計書 §4.2 の ⌘⌥F）。
    // gpui-kit の初期化より後に登録するので、同じ文脈の割り当てより優先される。
    if cfg!(target_os = "macos") {
        cx.bind_keys([KeyBinding::new("cmd-alt-f", gpui_kit::base::input::Replace, Some("Input"))]);
    }
}

/// Git で変更があるファイルは差分から開く（設計書 §5.3）。
pub(crate) fn initial_mode(mark: Option<GitMark>) -> FileMode {
    if has_diff(mark) { FileMode::Diff } else { FileMode::Full }
}

/// 差分／全体の切り替えを出すか（Git で変更があるファイルだけ）。
pub(crate) fn has_diff(mark: Option<GitMark>) -> bool {
    matches!(mark, Some(GitMark::Modified | GitMark::Added | GitMark::Deleted))
}

/// 保存が終わった時点のエディタの中身 current が、保存した中身 saved と違えば未保存のまま。
pub(crate) fn dirty_after_save(current: &str, saved: &str) -> bool {
    current != saved
}

/// Markdown の表示モード（ソースとプレビュー）とソースモード（設計書 §5.4）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MdMode {
    Preview,
    Source,
}

/// 本文の出し方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MdBody {
    Diff,
    Editor,
    /// ソースとプレビューを左右に並べる。
    Split,
    /// エディタ列が狭いので、プレビューだけ。
    PreviewOnly,
}

/// mode は、差分があって差分の表示中なら Diff、それ以外は Full を渡す。column_w はエディタ列の幅。
pub(crate) fn md_body(is_md: bool, mode: FileMode, md: MdMode, column_w: f32) -> MdBody {
    match (mode, is_md, md) {
        (FileMode::Diff, _, _) => MdBody::Diff,
        (_, true, MdMode::Preview) if split_preview(column_w) => MdBody::Split,
        (_, true, MdMode::Preview) => MdBody::PreviewOnly,
        _ => MdBody::Editor,
    }
}

pub(crate) fn open_error_text(e: OpenError) -> String {
    match e {
        OpenError::Binary => "バイナリのため開けない".into(),
        OpenError::NotUtf8 => "UTF-8 ではないため開けない".into(),
        OpenError::TooLarge(n) => format!("大きすぎるため開けない（{:.1} MB）", n as f64 / (1024.0 * 1024.0)),
    }
}

/// FileView から持ち主へ知らせること。
pub enum FileViewEvent {
    /// Esc が親まで来た。ターミナルへフォーカスを戻す。
    LeaveEditor,
    /// 未保存の状態が変わった。
    Dirty,
    Saved,
    /// キーでの保存に失敗した（知らせる文）。
    SaveFailed(String),
    /// このファイル（lines は選んだ行の範囲）を、最後に使ったターミナルの Claude Code に渡す。
    SendToClaude { rel: String, lines: Option<(u32, u32)> },
    /// プレビューのリンクから、プロジェクトの中のファイル（ルートからの相対パス）を開く。
    OpenRel(String),
}

/// 上部の帯。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Banner {
    /// 未保存の変更があるときに、外部で書き換えられた。
    ChangedOutside,
    /// ファイルが見つからない。
    Missing,
}

enum Body {
    Loading,
    Error(String),
    Text { doc: TextDoc, editor: Entity<EditorState>, gutter: GutterDecorationCollection },
}

pub struct FileView {
    fs: Arc<dyn ProjectFs>,
    root: ProjectRoot,
    pub(crate) rel: String,
    mode: FileMode,
    mark: Option<GitMark>,
    body: Body,
    diff: Option<Entity<DiffState>>,
    /// 最後に読んだ、または保存した中身（ディスク上のバイト列）。外部の変更と自分の保存を見分ける。
    disk: Option<Vec<u8>>,
    /// 最後に読んだ、または保存した時点のエディタの文字列。dirty の判定に使う。
    saved_text: String,
    dirty: bool,
    banner: Option<Banner>,
    /// 読み込みのあとに置くカーソルの位置（0 始まりの行と列）。
    pending_cursor: Option<(u32, u32)>,
    /// Git の状態を読む前に開いたため、表示の仕方を仮に決めている。状態が届いたら決め直す
    /// （利用者が切り替えたら、決め直さない）。
    auto_mode: bool,
    /// 読み込んで、まだエディタに入れていない中身。
    pending_doc: Option<TextDoc>,
    /// Markdown の表示モードかソースモードか。開くたびに表示モードから始める。
    pub(crate) md_mode: MdMode,
    /// Markdown のプレビュー。Markdown を読み込んだら作る。
    preview: Option<MdPreview>,
    /// エディタ列の幅（論理ピクセル）。表示モードで並べるかを決める。
    column_w: f32,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<FileViewEvent> for FileView {}

impl FileView {
    pub fn new(fs: Arc<dyn ProjectFs>, root: ProjectRoot, rel: String, mode: FileMode, mark: Option<GitMark>, cx: &mut Context<Self>) -> Self {
        let mut this = FileView {
            fs,
            root,
            rel,
            mode,
            mark,
            body: Body::Loading,
            diff: None,
            disk: None,
            saved_text: String::new(),
            dirty: false,
            banner: None,
            pending_cursor: None,
            pending_doc: None,
            auto_mode: false,
            md_mode: MdMode::Preview,
            preview: None,
            column_w: 1000.0,
            focus: cx.focus_handle(),
            _subs: Vec::new(),
        };
        this.load(cx);
        this
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Git の状態がまだ無いときに開いた（set_mark で表示の仕方を決め直す）。
    pub fn set_auto_mode(&mut self, auto: bool) {
        self.auto_mode = auto;
    }

    /// Git の記号が変わったら（状態の読み直し）、差分を読み直す。
    pub fn set_mark(&mut self, mark: Option<GitMark>, cx: &mut Context<Self>) {
        if self.auto_mode {
            self.auto_mode = false;
            if !self.dirty {
                self.mode = initial_mode(mark);
            }
        }
        if self.mark != mark {
            self.mark = mark;
            if !has_diff(mark) && self.mode == FileMode::Diff {
                self.mode = FileMode::Full;
            }
            self.load_diff(cx);
        }
    }

    /// 全体の表示にして、line（1 始まり）と col（1 始まり）にカーソルを置く。
    pub fn goto(&mut self, line: u32, col: Option<u32>, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = FileMode::Full;
        let pos = (line.saturating_sub(1), col.unwrap_or(1).saturating_sub(1));
        match &self.body {
            Body::Text { editor, .. } => {
                editor.update(cx, |e, cx| e.set_cursor_position(Position::new(pos.0, pos.1), window, cx));
            }
            _ => self.pending_cursor = Some(pos),
        }
        cx.notify();
    }

    /// ツリーで名前が変わった。新しいパスで読み直しの要否を確かめる（見つからないの帯を消すため）。
    pub fn set_rel(&mut self, rel: String, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.preview {
            p.set_md_rel(rel.clone());
        }
        self.rel = rel;
        self.on_external_change(cx);
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: FileMode, cx: &mut Context<Self>) {
        self.auto_mode = false;
        if mode == FileMode::Full && self.mark == Some(GitMark::Deleted) {
            return;
        }
        self.mode = mode;
        cx.notify();
    }

    /// エディタ列の幅。描くたびに持ち主が渡す。
    pub(crate) fn set_column_width(&mut self, w: Pixels) {
        self.column_w = f32::from(w);
    }

    fn is_md(&self) -> bool {
        is_markdown(&self.rel)
    }

    /// 差分を出しているか（差分の表示中で、差分がある）。
    fn showing_diff(&self) -> bool {
        self.mode == FileMode::Diff && self.diff.is_some()
    }

    /// 表示モードかソースモードにする。差分の表示中なら、全体の表示に切り替えてから。
    pub(crate) fn set_md_mode(&mut self, md: MdMode, cx: &mut Context<Self>) {
        if self.mode == FileMode::Diff {
            self.set_mode(FileMode::Full, cx);
        }
        self.md_mode = md;
        cx.notify();
    }

    /// ⌘E / Ctrl+E。差分の表示中は全体の表示にするだけ（今のモードのまま）。Markdown でなければ何もしない。
    pub(crate) fn toggle_md_mode(&mut self, cx: &mut Context<Self>) {
        if !self.is_md() {
            return;
        }
        if self.showing_diff() {
            self.set_mode(FileMode::Full, cx);
            return;
        }
        let next = if self.md_mode == MdMode::Preview { MdMode::Source } else { MdMode::Preview };
        self.set_md_mode(next, cx);
    }

    /// 本文の出し方。
    fn layout(&self) -> MdBody {
        let mode = if self.showing_diff() { FileMode::Diff } else { FileMode::Full };
        md_body(self.is_md() && self.preview.is_some(), mode, self.md_mode, self.column_w)
    }

    /// Markdown のテキストを読み込んだら、プレビューを作るか中身を替える。
    fn sync_preview(&mut self, cx: &mut Context<Self>) {
        let Body::Text { editor, .. } = &self.body else { return };
        if !self.is_md() {
            return;
        }
        let text = editor.read(cx).value().to_string();
        match &self.preview {
            Some(p) => p.set_text(&text, cx),
            None => self.preview = Some(MdPreview::new(&text, self.root.clone(), self.rel.clone(), cx)),
        }
    }

    fn rel_path(&self) -> io::Result<RelPath> {
        RelPath::new(&self.rel).map_err(io::Error::from)
    }

    /// ファイルと差分を裏で読む。
    fn load(&mut self, cx: &mut Context<Self>) {
        let fs = self.fs.clone();
        let rel = self.rel_path();
        let task = cx.background_spawn(async move { fs.read(&rel?) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.loaded(result, cx)).ok();
        })
        .detach();
        self.load_diff(cx);
    }

    fn load_diff(&mut self, cx: &mut Context<Self>) {
        if !has_diff(self.mark) {
            self.diff = None;
            self.update_gutter(&[], cx);
            return;
        }
        let fs = self.fs.clone();
        let rel = self.rel_path();
        let task = cx.background_spawn(async move { fs.git_diff(&rel?) });
        cx.spawn(async move |this, cx| {
            let patch = task.await;
            this.update(cx, |this, cx| match patch {
                Ok(patch) => this.diff_loaded(&patch, cx),
                Err(e) => eprintln!("差分を読めない: {}: {e}", this.rel),
            })
            .ok();
        })
        .detach();
    }

    fn diff_loaded(&mut self, patch: &str, cx: &mut Context<Self>) {
        let marks = line_marks(patch);
        self.update_gutter(&marks, cx);
        match DiffFile::parse(patch) {
            Ok(files) if !files.is_empty() => match &self.diff {
                Some(state) => state.update(cx, |s, cx| s.set_files(files, cx)),
                None => self.diff = Some(cx.new(|cx| DiffState::new(files, cx))),
            },
            // 差分が空（保存して HEAD と同じになった、など）なら全体の表示にする。
            Ok(_) => {
                self.diff = None;
                if self.mode == FileMode::Diff && self.mark != Some(GitMark::Deleted) {
                    self.mode = FileMode::Full;
                }
            }
            Err(e) => eprintln!("差分を解釈できない: {}: {e:?}", self.rel),
        }
        cx.notify();
    }

    /// 変更された行の左端の印（§5.3）。Added と Modified は diff.add、DeletedAbove は diff.del。
    fn update_gutter(&mut self, marks: &[LineMark], cx: &mut Context<Self>) {
        let Body::Text { editor, gutter, .. } = &self.body else { return };
        let theme = *cx.global::<Theme>();
        let text = editor.read(cx).value();
        let starts: Vec<usize> =
            std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
        let line_range = |line: u32| -> Option<std::ops::Range<usize>> {
            let i = (line as usize).checked_sub(1)?;
            let start = *starts.get(i)?;
            let end = starts.get(i + 1).map_or(text.len(), |e| e.saturating_sub(1));
            Some(start..end.max(start))
        };
        let decorations = marks
            .iter()
            .filter_map(|m| {
                let (line, color) = match *m {
                    LineMark::Added(l) | LineMark::Modified(l) => (l, theme.diff_add),
                    LineMark::DeletedAbove(l) => (l, theme.diff_del),
                };
                line_range(line).map(|r| GutterDecoration::new(r).with_color(color.into()))
            })
            .collect();
        gutter.set(decorations, cx);
    }

    fn loaded(&mut self, result: io::Result<Vec<u8>>, cx: &mut Context<Self>) {
        let bytes = match result {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound && self.mark == Some(GitMark::Deleted) => {
                // 削除したファイルは差分だけを出す。
                self.body = Body::Error("削除されたファイル".into());
                cx.notify();
                return;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.body = Body::Error("ファイルが見つからない".into());
                cx.notify();
                return;
            }
            Err(e) => {
                self.body = Body::Error(format!("読めない: {e}"));
                cx.notify();
                return;
            }
        };
        let doc = match decode_text(&bytes) {
            Ok(doc) => doc,
            Err(e) => {
                self.body = Body::Error(open_error_text(e));
                cx.notify();
                return;
            }
        };
        self.disk = Some(bytes);
        self.saved_text = doc.text.clone();
        // エディタを作るにはウィンドウが要るため、次の描画で入れる。
        self.pending_doc = Some(doc);
        cx.notify();
    }

    /// 読み込んだ中身をエディタに入れる。初回はエディタを作り、2 回目以降（読み直し）は中身だけを替える。
    fn install_text(&mut self, doc: TextDoc, window: &mut Window, cx: &mut Context<Self>) {
        if let Body::Text { editor, doc: old, .. } = &mut self.body {
            let pos = editor.read(cx).cursor_position();
            editor.update(cx, |e, cx| {
                e.set_value(doc.text.clone(), window, cx);
                e.set_cursor_position(pos, window, cx);
            });
            *old = doc;
        } else {
            let text = doc.text.clone();
            let lang = language_for(&self.rel);
            let editor = cx.new(|cx| {
                let mut state = EditorState::new(window, cx).line_number(true).searchable(true);
                if let Some(lang) = lang {
                    state = state.language(lang);
                }
                state.set_value(text, window, cx);
                state
            });
            // 選択が変わったら、選択中の小さなボタンを描き直す。
            self._subs.push(cx.observe(&editor, |_, _, cx| cx.notify()));
            let sub = cx.subscribe(&editor, |this, editor, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    let dirty = editor.read(cx).value() != this.saved_text;
                    if dirty != this.dirty {
                        this.dirty = dirty;
                        cx.emit(FileViewEvent::Dirty);
                    }
                    this.sync_preview(cx);
                    cx.notify();
                }
            });
            self._subs.push(sub);
            let gutter = editor.update(cx, |e, cx| e.create_gutter_decorations_collection(Vec::new(), cx));
            self.body = Body::Text { doc, editor, gutter };
        }
        if let (Some((line, col)), Body::Text { editor, .. }) = (self.pending_cursor.take(), &self.body) {
            editor.update(cx, |e, cx| e.set_cursor_position(Position::new(line, col), window, cx));
        }
        self.dirty = false;
        self.banner = None;
        self.sync_preview(cx);
        self.load_diff(cx);
        cx.notify();
    }

    /// 保存する。改行と BOM は開いたときの形のまま。書き終えたら結果を返す。
    pub fn save(&mut self, cx: &mut Context<Self>) -> Task<io::Result<()>> {
        let Body::Text { doc, editor, .. } = &self.body else { return Task::ready(Ok(())) };
        let text = editor.read(cx).value().to_string();
        let bytes = encode_text(doc, &text);
        let fs = self.fs.clone();
        let rel = self.rel_path();
        let written = bytes.clone();
        let task = cx.background_spawn(async move { fs.write(&rel?, &written) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            if result.is_ok() {
                this.update(cx, |this, cx| {
                    // 書いている間に打った文字は、保存した中身に入っていない。
                    let current = match &this.body {
                        Body::Text { editor, .. } => editor.read(cx).value().to_string(),
                        _ => text.clone(),
                    };
                    this.disk = Some(bytes);
                    this.dirty = dirty_after_save(&current, &text);
                    this.saved_text = text;
                    this.banner = None;
                    cx.emit(FileViewEvent::Dirty);
                    cx.emit(FileViewEvent::Saved);
                    this.load_diff(cx);
                    cx.notify();
                })
                .ok();
            }
            result
        })
    }

    /// 全体の表示で選んでいる行（1 始まり）。差分の表示や、選んでいないときは None。
    fn selected_lines(&self, cx: &App) -> Option<(u32, u32)> {
        let Body::Text { editor, .. } = &self.body else { return None };
        if self.mode == FileMode::Diff && self.diff.is_some() {
            return None;
        }
        let e = editor.read(cx);
        selected_lines(&e.value(), e.selected_range())
    }

    /// 上部のボタン、選択中の小さなボタン、⌘⇧L / Ctrl+Shift+L から。
    fn send_to_claude(&mut self, cx: &mut Context<Self>) {
        let lines = self.selected_lines(cx);
        cx.emit(FileViewEvent::SendToClaude { rel: self.rel.clone(), lines });
    }

    /// 選択の右上に重ねる小さなボタン。選択が空か、見えていないときは出さない。
    fn render_selection_button(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let Body::Text { editor, .. } = &self.body else { return None };
        if matches!(self.layout(), MdBody::Diff | MdBody::PreviewOnly) {
            return None;
        }
        let e = editor.read(cx);
        let range = e.selected_range();
        if range.is_empty() {
            return None;
        }
        let bounds = e.range_to_bounds(&range)?;
        let right = bounds.left().max(bounds.right());
        let top = bounds.top().min(bounds.bottom());
        let button = div()
            .id("fv-send-selection")
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(size::RADIUS_SM)
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border_subtle)
            .shadow(popup_shadow())
            .hover(|s| s.bg(theme.bg_hover))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| this.send_to_claude(cx)))
            .child(icon(IconName::Send, theme.fg_default));
        let position = point(right + px(4.), top - px(22.));
        Some(deferred(anchored().position(position).child(button)).with_priority(1))
    }

    /// キー（⌘S / Ctrl+S）での保存。失敗したら持ち主に知らせる。
    fn save_from_key(&mut self, cx: &mut Context<Self>) {
        let task = self.save(cx);
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                this.update(cx, |this, cx| {
                    let message = error_text("保存できません", &this.rel, &e);
                    cx.emit(FileViewEvent::SaveFailed(message));
                })
                .ok();
            }
        })
        .detach();
    }

    /// 監視からの「このファイルが変わった」。自分の保存と同じ中身なら何もしない。
    pub fn on_external_change(&mut self, cx: &mut Context<Self>) {
        let fs = self.fs.clone();
        let rel = self.rel_path();
        let task = cx.background_spawn(async move { fs.read(&rel?) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let outcome = match &result {
                    Ok(bytes) if this.disk.as_ref() == Some(bytes) => ReadOutcome::Same,
                    Ok(_) => ReadOutcome::Changed,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => ReadOutcome::Missing,
                    Err(_) => ReadOutcome::Failed,
                };
                match external_change(this.dirty, outcome) {
                    ExternalAction::Reload => this.loaded(result, cx),
                    ExternalAction::AskKeepOrReload => this.banner = Some(Banner::ChangedOutside),
                    ExternalAction::MarkMissing => this.banner = Some(Banner::Missing),
                    // 同じ中身で戻ってきた（名前の変更、stash など）なら、見つからないの帯を消す。
                    ExternalAction::Ignore if outcome == ReadOutcome::Same && this.banner == Some(Banner::Missing) => {
                        this.banner = None;
                    }
                    ExternalAction::Ignore => return,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_header(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let name = self.rel.rsplit('/').next().unwrap_or(&self.rel).to_string();
        let parent = self.rel.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        let button = |id: &'static str, name: IconName, enabled: bool| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(size::RADIUS_SM)
                .hover(|s| s.bg(theme.bg_hover))
                .child(icon(name, if enabled { theme.fg_default } else { Rgba { a: 0.35, ..theme.fg_dim } }))
        };
        let rel = self.rel.clone();
        let mut row = div()
            .h(px(36.))
            .px(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(
                div()
                    .id("fv-name")
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(6.))
                    .child(div().flex_shrink_0().text_color(theme.fg_default).child(name))
                    // 未保存の印。場所は常に取っておき、幅を変えない。
                    .child(div().size(px(6.)).flex_shrink_0().rounded_full().when(self.dirty, |d| d.bg(theme.accent)))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_color(theme.fg_dim).child(parent))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(rel.clone()))),
            )
            .child(div().flex_1());
        let seg = |id: &'static str, label: &'static str, on: bool| {
                div()
                    .id(id)
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(5.))
                    .text_size(px(12.))
                    .when(on, |d| d.bg(theme.bg_hover).text_color(theme.fg_default))
                    .when(!on, |d| d.text_color(theme.fg_dim))
                    .child(label)
        };
        let segments = |a: Stateful<Div>, b: Stateful<Div>| {
            div().flex().flex_row().p(px(2.)).rounded(size::RADIUS_SM).border_1().border_color(theme.border_subtle).child(a).child(b)
        };
        if has_diff(self.mark) {
            let diff_on = self.mode == FileMode::Diff;
            row = row.child(segments(
                seg("fv-diff", "差分", diff_on).on_click(cx.listener(|this, _, _, cx| this.set_mode(FileMode::Diff, cx))),
                seg("fv-full", "全体", !diff_on).on_click(cx.listener(|this, _, _, cx| this.set_mode(FileMode::Full, cx))),
            ));
            if diff_on && let Some(state) = self.diff.clone() {
                let prev = state.clone();
                row = row
                    .child(button("fv-prev", IconName::ArrowUp, true).on_click(cx.listener(move |_, _, _, cx| {
                        prev.update(cx, |s, cx| s.previous_change(cx));
                    })))
                    .child(button("fv-next", IconName::ArrowDown, true).on_click(cx.listener(move |_, _, _, cx| {
                        state.update(cx, |s, cx| s.next_change(cx));
                    })));
            }
        }
        if self.is_md() {
            // 差分の表示中は、どちらも選んでいない見た目にする。
            let shown = !self.showing_diff();
            let preview_on = shown && self.md_mode == MdMode::Preview;
            let source_on = shown && self.md_mode == MdMode::Source;
            row = row.child(segments(
                seg("fv-md-preview", "表示", preview_on)
                    .on_click(cx.listener(|this, _, _, cx| this.set_md_mode(MdMode::Preview, cx))),
                seg("fv-md-source", "ソース", source_on)
                    .on_click(cx.listener(|this, _, _, cx| this.set_md_mode(MdMode::Source, cx))),
            ));
        }
        row.child(
            button("fv-send", IconName::Send, true)
                .on_click(cx.listener(|this, _, _, cx| this.send_to_claude(cx))),
        )
    }

    /// プレビューの要素（文字は UI のフォント）。プレビューが無ければ None。
    fn render_preview(&self, cx: &mut Context<Self>) -> Option<Div> {
        let preview = self.preview.as_ref()?;
        let this = cx.entity().downgrade();
        let link = cx.global::<Theme>().accent.into();
        let view = preview.element(
            link,
            move |rel, _, cx| {
                this.update(cx, |_, cx| cx.emit(FileViewEvent::OpenRel(rel))).ok();
            },
            cx,
        );
        Some(div().size_full().pl(px(16.)).pr(px(8.)).pt(px(8.)).font(ui_font()).text_size(px(14.)).child(view))
    }

    fn render_banner(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let banner = self.banner?;
        let text = match banner {
            Banner::ChangedOutside => "外部で変更されました",
            Banner::Missing => "ファイルが見つかりません",
        };
        let action = |id: &'static str, label: &'static str| {
            div().id(id).px(px(8.)).h(px(22.)).flex().items_center().rounded(px(5.)).hover(|s| s.bg(theme.bg_hover)).child(label)
        };
        let mut row = div()
            .h(px(30.))
            .px(px(12.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.border_subtle)
            .text_size(px(12.))
            .text_color(theme.warn)
            .child(div().flex_1().child(text));
        if banner == Banner::ChangedOutside {
            row = row
                .child(action("fv-reload", "読み直す").text_color(theme.fg_default).on_click(cx.listener(|this, _, _, cx| {
                    this.dirty = false;
                    this.banner = None;
                    cx.emit(FileViewEvent::Dirty);
                    this.load(cx);
                })))
                .child(action("fv-keep", "このまま").text_color(theme.fg_dim).on_click(cx.listener(|this, _, _, cx| {
                    this.banner = None;
                    cx.notify();
                })));
        }
        Some(row)
    }
}

impl Render for FileView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(doc) = self.pending_doc.take() {
            self.install_text(doc, window, cx);
        }
        let theme = *cx.global::<Theme>();
        let header = self.render_header(&theme, cx);
        let banner = self.render_banner(&theme, cx);
        let send = self.render_selection_button(&theme, cx);
        let layout = self.layout();
        let body: AnyElement = match (layout, &self.diff, &self.body) {
            (MdBody::Diff, Some(state), _) => Diff::new(state).into_any_element(),
            (_, _, Body::Text { editor, .. }) => {
                let source = || Editor::new(editor).bordered(false).appearance(false).h_full();
                let preview = self.render_preview(cx);
                match (layout, preview) {
                    (MdBody::Split, Some(preview)) => div()
                        .size_full()
                        .flex()
                        .flex_row()
                        .child(div().flex_1().min_w_0().h_full().child(source()))
                        .child(div().w(px(1.)).h_full().flex_shrink_0().bg(theme.border_subtle))
                        .child(div().flex_1().min_w_0().h_full().child(preview))
                        .into_any_element(),
                    (MdBody::PreviewOnly, Some(preview)) => preview.into_any_element(),
                    _ => source().into_any_element(),
                }
            }
            (_, _, Body::Error(msg)) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_dim)
                .child(msg.clone())
                .into_any_element(),
            (_, _, Body::Loading) => div().size_full().into_any_element(),
        };
        div()
            .id("file-view")
            .track_focus(&self.focus)
            .key_context("FileView")
            .on_drop(|_: &ExternalPaths, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|this, _: &SaveFile, _, cx| this.save_from_key(cx)))
            .on_action(cx.listener(|this, _: &SendToClaude, _, cx| this.send_to_claude(cx)))
            .on_action(cx.listener(|this, _: &ToggleMarkdownMode, _, cx| this.toggle_md_mode(cx)))
            // エディタは、閉じる検索欄や選択が無いときに Esc を親へ回す。そのときだけターミナルへ戻る。
            .on_action(cx.listener(|_, _: &gpui_kit::base::input::Escape, _, cx| cx.emit(FileViewEvent::LeaveEditor)))
            .on_action(cx.listener(|_, _: &LeaveEditor, _, cx| cx.emit(FileViewEvent::LeaveEditor)))
            .size_full()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .child(header)
            .children(banner)
            .child(div().flex_1().min_h_0().font(mono_font()).child(body))
            .children(send)
    }
}

#[cfg(test)]
mod tests {
    use super::{MdBody, MdMode, dirty_after_save, has_diff, initial_mode, md_body, open_error_text};
    use fude_core::panel::FileMode;
    use fude_core::text::OpenError;
    use fude_fs::git::GitMark;

    #[test]
    fn initial_mode_rules() {
        assert_eq!(initial_mode(Some(GitMark::Modified)), FileMode::Diff);
        assert_eq!(initial_mode(Some(GitMark::Added)), FileMode::Diff);
        assert_eq!(initial_mode(Some(GitMark::Deleted)), FileMode::Diff);
        assert_eq!(initial_mode(Some(GitMark::Untracked)), FileMode::Full);
        assert_eq!(initial_mode(None), FileMode::Full);
    }

    #[test]
    fn typing_during_save_stays_dirty() {
        // 保存の書き込み中に打った文字は、保存した中身に入っていないので未保存のまま。
        assert!(dirty_after_save("abcd", "abc"));
        assert!(!dirty_after_save("abc", "abc"));
    }

    #[test]
    fn diff_toggle_shows_only_for_changes() {
        assert!(has_diff(Some(GitMark::Modified)));
        assert!(!has_diff(Some(GitMark::Untracked)));
        assert!(!has_diff(None));
    }

    #[test]
    fn open_error_messages() {
        assert_eq!(open_error_text(OpenError::Binary), "バイナリのため開けない");
        assert_eq!(open_error_text(OpenError::NotUtf8), "UTF-8 ではないため開けない");
        assert_eq!(open_error_text(OpenError::TooLarge(9 * 1024 * 1024)), "大きすぎるため開けない（9.0 MB）");
    }

    #[test]
    fn markdown_body_layout() {
        assert_eq!(md_body(false, FileMode::Full, MdMode::Preview, 800.0), MdBody::Editor);
        assert_eq!(md_body(true, FileMode::Full, MdMode::Preview, 800.0), MdBody::Split);
        assert_eq!(md_body(true, FileMode::Full, MdMode::Preview, 500.0), MdBody::PreviewOnly);
        assert_eq!(md_body(true, FileMode::Full, MdMode::Source, 800.0), MdBody::Editor);
        assert_eq!(md_body(true, FileMode::Diff, MdMode::Preview, 800.0), MdBody::Diff);
        assert_eq!(md_body(false, FileMode::Diff, MdMode::Preview, 800.0), MdBody::Diff);
    }
}
