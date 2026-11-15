//! ファイルツリーの左パネル（設計書 §4.1、§4.3、§5.5）。レールの右に、メイン領域の上へ重ねて出す。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use fude_core::workspace::ProjectId;
use fude_fs::RelPath;
use fude_fs::watch::WatchBatch;
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::changes_menu::mark_style;
use crate::file_icons::{file_icon, icon_for_file, icon_for_folder};
use crate::theme::{Theme, mono_font, size};
use crate::title_row::icon;
use crate::tree_model::{RowSlot, TreeModel, TreeRow, rows_with_edit};
use crate::workspace_view::WorkspaceView;

gpui_kit::actions!(
    tree,
    [
        ToggleTree,
        CloseTree,
        TreeUndo,
        TreeRedo,
        TreeUp,
        TreeDown,
        TreeHome,
        TreeEnd,
        TreePageUp,
        TreePageDown,
        TreeLeft,
        TreeRight,
        TreeOpen,
        TreeRename,
        TreeDelete,
        TreeMenu,
        TreeFilterDown,
        TreeFilterCancel
    ]
);

/// 絞り込み欄の中の文脈。
const TREE_FILTER_INPUT: &str = "TreeFilter > Input";

/// ツリーを閉じたときのフォーカスの行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseFocus {
    /// 選んでいるエディタのタブへ。
    Editor,
    /// 開く前の場所へ（restore_focus）。
    Restore,
}

/// ツリーからファイルを開いていて、エディタのタブがあれば、閉じたらエディタへ移る。
fn focus_after_close(opened_file: bool, has_editor: bool) -> CloseFocus {
    if opened_file && has_editor { CloseFocus::Editor } else { CloseFocus::Restore }
}


/// ツリーの取り消しとやり直しが効く文脈。入力欄と絞り込み欄の中では、入力欄の取り消しを優先する。
const TREE_NOT_INPUT: &str = "Tree && !Input";

/// ツリーの開閉（⌘B、Windows は Ctrl+B。ターミナルの中では Ctrl+Shift+B。設計書 §10）と、Esc で閉じる。
/// 取り消しとやり直し（計画8b）は ⌘Z / ⌘⇧Z、Windows は Ctrl+Z / Ctrl+Shift+Z。
pub fn bind_tree_keys(cx: &mut App) {
    // 開閉のキーは fude_core::keymap（workspace_view::bind_keys）で登録する。
    let (undo, redo, delete) = match fude_core::keymap::Platform::current() {
        fude_core::keymap::Platform::MacOs => ("cmd-z", "cmd-shift-z", "cmd-backspace"),
        fude_core::keymap::Platform::Windows => ("ctrl-z", "ctrl-shift-z", "delete"),
    };
    cx.bind_keys([
        KeyBinding::new("escape", CloseTree, Some("Tree")),
        KeyBinding::new(undo, TreeUndo, Some(TREE_NOT_INPUT)),
        KeyBinding::new(redo, TreeRedo, Some(TREE_NOT_INPUT)),
        // キーでの操作（計画8c）。入力欄の中では入力欄が受け取る。
        KeyBinding::new("up", TreeUp, Some(TREE_NOT_INPUT)),
        KeyBinding::new("down", TreeDown, Some(TREE_NOT_INPUT)),
        KeyBinding::new("home", TreeHome, Some(TREE_NOT_INPUT)),
        KeyBinding::new("end", TreeEnd, Some(TREE_NOT_INPUT)),
        KeyBinding::new("pageup", TreePageUp, Some(TREE_NOT_INPUT)),
        KeyBinding::new("pagedown", TreePageDown, Some(TREE_NOT_INPUT)),
        KeyBinding::new("left", TreeLeft, Some(TREE_NOT_INPUT)),
        KeyBinding::new("right", TreeRight, Some(TREE_NOT_INPUT)),
        KeyBinding::new("enter", TreeOpen, Some(TREE_NOT_INPUT)),
        KeyBinding::new("f2", TreeRename, Some(TREE_NOT_INPUT)),
        KeyBinding::new(delete, TreeDelete, Some(TREE_NOT_INPUT)),
        KeyBinding::new("shift-f10", TreeMenu, Some(TREE_NOT_INPUT)),
        // 絞り込み欄の中。同じ深さで後から登録したものが、gpui-kit の入力欄のキーより勝つ。
        KeyBinding::new("down", TreeFilterDown, Some(TREE_FILTER_INPUT)),
        KeyBinding::new("escape", TreeFilterCancel, Some(TREE_FILTER_INPUT)),
    ]);
    // Windows のメニューキー（VK_APPS）は GPUI では "menu" として届く。
    if fude_core::keymap::Platform::current() == fude_core::keymap::Platform::Windows {
        cx.bind_keys([KeyBinding::new("menu", TreeMenu, Some(TREE_NOT_INPUT))]);
    }
}

pub(crate) const ROW_H: Pixels = px(24.);
/// 監視の通知のあと、全ファイルの一覧を読み直すまでの待ち時間（続けて届いたら数え直す）。
const FILE_LIST_DELAY: Duration = Duration::from_millis(500);
const INDENT: f32 = 12.;

/// 左パネルの状態。
pub(crate) struct TreeState {
    pub models: HashMap<ProjectId, TreeModel>,
    pub open: bool,
    pub filter: Option<Entity<InputState>>,
    pub focus: FocusHandle,
    /// 最後の描画で作った行（一覧の部品が範囲ごとに読む）。名前の入力欄も行として入る。
    pub rows: Vec<RowSlot>,
    pub scroll: UniformListScrollHandle,
    /// 次の描画で、名前の入力欄の行が見える位置までスクロールする。
    pub scroll_to_edit: bool,
    /// 選択中の行（クリックかキーで選んだもの）。並びに無ければ、無いものとして扱う。
    pub selected: Option<RelPath>,
    /// 次の描画で、選択の行が見える位置までスクロールする（並びに現れるまで持ち越す）。
    pub scroll_to_selected: bool,
    /// 行の一覧の位置と大きさ（描画のたびに測る）。
    pub list_bounds: Bounds<Pixels>,
    /// 右クリックメニューと、その対象の行（None は何も無いところ）。
    pub menu: Option<crate::tree_menu::TreeMenuState>,
    /// 新規や名前の変更で、名前を入れている途中。
    pub edit: Option<crate::tree_menu::TreeEdit>,
    /// ツリーを開いてからファイルを開いたか。閉じたときにエディタへ移るかを決める。
    pub opened_file: bool,
    /// プロジェクトごとの取り消しとやり直しの履歴（メモリだけ）。
    pub history: HashMap<ProjectId, fude_core::tree_history::TreeHistory<fude_fs::TrashTicket>>,
    /// 全ファイルの一覧を読んでいる途中（待ち合わせを含む）。差し替えると前の分は捨てられる。
    pub file_list_task: Option<Task<()>>,
    _filter_sub: Option<Subscription>,
}

impl TreeState {
    pub fn new(cx: &mut App) -> TreeState {
        TreeState {
            models: HashMap::new(),
            open: false,
            filter: None,
            focus: cx.focus_handle(),
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            scroll_to_edit: false,
            selected: None,
            scroll_to_selected: false,
            list_bounds: Bounds::default(),
            menu: None,
            edit: None,
            history: HashMap::new(),
            file_list_task: None,
            opened_file: false,
            _filter_sub: None,
        }
    }
}

impl WorkspaceView {
    pub(crate) fn toggle_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.open {
            self.close_tree(window, cx);
            return;
        }
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        self.tree.open = true;
        self.tree.opened_file = false;
        if self.tree.filter.is_none() {
            let state = cx.new(|cx| InputState::new(window, cx));
            let sub = cx.subscribe_in(&state, window, |this, state, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let query = state.read(cx).value().to_string();
                    this.set_tree_filter(query, cx);
                    // ツリーの本体で打った文字（本体は欄を入力の受け手にしている）なら、そのまま欄へ移る。
                    if this.tree.focus.is_focused(window) {
                        state.update(cx, |s, cx| s.focus(window, cx));
                    }
                }
                // 絞り込み欄の Enter は、先頭のファイルの候補を開く。
                InputEvent::PressEnter { .. } => {
                    let first = crate::tree_nav::first_file(&this.tree.rows);
                    if let (Some(rel), Some(project)) = (first, this.workspace.active_project().map(|p| p.id)) {
                        this.open_tree_file(project, rel, window, cx);
                    }
                }
                _ => {}
            });
            self.tree.filter = Some(state);
            self.tree._filter_sub = Some(sub);
        }
        if self.tree.models.entry(project).or_default().needs_load(&RelPath::root()) {
            self.load_tree_dir(project, RelPath::root(), cx);
        }
        self.select_active_file(project, cx);
        window.focus(&self.tree.focus, cx);
        cx.notify();
    }

    /// 開いているファイルの行を選ぶ。親フォルダが閉じていれば開いて読む。絞り込み中と「変更だけ」の表示では、
    /// 並びにあるときだけ選ぶ。全体検索のタブやタブが無いときは、今の選択のまま（並びに無ければ ↓ で先頭へ）。
    fn select_active_file(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let active = self.workspace.active_project().and_then(|p| p.editor.active()).filter(|i| !i.is_virtual());
        let Some(rel) = active.and_then(|i| RelPath::new(&i.rel).ok()) else { return };
        let model = self.tree.models.entry(project).or_default();
        if model.filtering() || model.changed_only() {
            if crate::tree_nav::selected_row(&self.tree.rows, Some(&rel)).is_some() {
                self.select_tree_row(Some(rel), cx);
            }
            return;
        }
        let parent = rel.as_str().rsplit_once('/').and_then(|(p, _)| RelPath::new(p).ok());
        for dir in parent.map(|p| model.reveal(&p)).unwrap_or_default() {
            self.load_tree_dir(project, dir, cx);
        }
        self.select_tree_row(Some(rel), cx);
    }

    /// 選択を入れ替え、次の描画でその行まで送る。
    pub(crate) fn select_tree_row(&mut self, rel: Option<RelPath>, cx: &mut Context<Self>) {
        self.tree.scroll_to_selected = rel.is_some();
        self.tree.selected = rel;
        cx.notify();
    }

    /// ファイルをエディタで開く。ツリーは開いたまま、フォーカスもツリーに残す（続けて探せるように）。
    /// エディタへはツリーを閉じると移る（focus_after_close）。
    pub(crate) fn open_tree_file(&mut self, project: ProjectId, rel: RelPath, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tree_row(Some(rel.clone()), cx);
        self.open_in_editor(project, rel.as_str().to_string(), None, None, window, cx);
        // 新しく作るビューは、作り終えたときにフォーカスを取る。その頼みを取り消し、ツリーに戻す。
        if let Some(slot) = self.editors.get_mut(&project) {
            slot.take_focus_request(rel.as_str());
        }
        self.tree.opened_file = true;
        window.focus(&self.tree.focus, cx);
        cx.notify();
    }

    fn tree_nav(&mut self, key: crate::tree_nav::NavKey, cx: &mut Context<Self>) {
        let page = crate::tree_nav::page_rows(f32::from(self.tree.list_bounds.size.height), f32::from(ROW_H));
        let next = crate::tree_nav::move_selection(&self.tree.rows, self.tree.selected.as_ref(), key, page);
        if next.is_some() {
            self.select_tree_row(next, cx);
        }
    }

    /// ← と →。絞り込み中と「変更だけ」の表示では何もしない（クリックでも開閉できないため）。
    fn tree_horizontal(&mut self, right: bool, cx: &mut Context<Self>) {
        use crate::tree_nav::Horizontal;
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        let model = self.tree.models.entry(project).or_default();
        if model.filtering() || model.changed_only() {
            return;
        }
        match crate::tree_nav::horizontal(&self.tree.rows, self.tree.selected.as_ref(), right, true) {
            Horizontal::Expand(dir) | Horizontal::Collapse(dir) => {
                if model.toggle(&dir) && model.needs_load(&dir) {
                    self.load_tree_dir(project, dir, cx);
                }
                cx.notify();
            }
            Horizontal::Select(rel) => self.select_tree_row(Some(rel), cx),
            Horizontal::Nothing => {}
        }
    }

    /// 選択の行。並びに無ければ None。
    fn selected_tree_row(&self) -> Option<TreeRow> {
        crate::tree_nav::selected_row(&self.tree.rows, self.tree.selected.as_ref()).map(|(_, row)| row.clone())
    }

    /// Enter。フォルダは開閉、ファイルは開いてツリーを閉じる。
    fn tree_open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected_tree_row() else { return };
        if row.is_dir {
            self.click_row(row, window, cx);
        } else if let Some(project) = self.workspace.active_project().map(|p| p.id) {
            self.open_tree_file(project, row.rel, window, cx);
        }
    }

    /// 絞り込み欄からリストへ戻る。↓ は先頭の行を選ぶ。Esc は欄が空でなければ空にし、空ならツリーを閉じる。
    fn leave_tree_filter(&mut self, cancel: bool, window: &mut Window, cx: &mut Context<Self>) {
        if cancel {
            let Some(filter) = self.tree.filter.clone() else { return };
            if filter.read(cx).value().is_empty() {
                self.close_tree(window, cx);
                return;
            }
            filter.update(cx, |s, cx| s.replace_all("", window, cx));
        } else {
            let first = crate::tree_nav::move_selection(&self.tree.rows, None, crate::tree_nav::NavKey::Down, 1);
            self.select_tree_row(first, cx);
        }
        window.focus(&self.tree.focus, cx);
        cx.notify();
    }

    fn tree_action_on_selected(&mut self, action: crate::tree_menu::TreeAction, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.selected_tree_row() {
            self.run_tree_action(Some(row), action, window, cx);
        }
    }

    pub(crate) fn close_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tree.open {
            return;
        }
        self.tree.open = false;
        let editor = self.active_editor_focus(cx);
        match (focus_after_close(std::mem::take(&mut self.tree.opened_file), editor.is_some()), editor) {
            (CloseFocus::Editor, Some(focus)) => self.focus_editor(&focus, window, cx),
            _ => self.restore_focus(window, cx),
        }
        cx.notify();
    }

    /// dir の子を裏で読む。ProjectFs がまだ無ければ、用意できたとき（tree_live_ready）に読む。
    pub(crate) fn load_tree_dir(&mut self, project: ProjectId, dir: RelPath, cx: &mut Context<Self>) {
        let Some(fs) = self.live_fs(project) else { return };
        let task = cx.background_spawn({
            let dir = dir.clone();
            async move { fs.list_dir(&dir) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(entries) => this.tree.models.entry(project).or_default().set_children(&dir, entries),
                    Err(e) => eprintln!("フォルダを読めない: {}: {e}", dir.as_str()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// ProjectFs が用意できたときに呼ぶ。ツリーが開いていれば読み始める。
    pub(crate) fn tree_live_ready(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        if !self.tree.open {
            return;
        }
        let model = self.tree.models.entry(project).or_default();
        let root = model.needs_load(&RelPath::root()).then(RelPath::root);
        // 用意できる前に開いたフォルダ（開いているファイルの祖先など）も読む。
        let dirs: Vec<RelPath> = root.into_iter().chain(model.unloaded_expanded()).collect();
        for dir in dirs {
            self.load_tree_dir(project, dir, cx);
        }
    }

    fn set_tree_filter(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        let model = self.tree.models.entry(project).or_default();
        model.set_filter(&query, None);
        if model.filtering() && !model.has_file_list() && self.tree.file_list_task.is_none() {
            self.load_file_list(project, Duration::ZERO, cx);
        }
        cx.notify();
    }

    /// 全ファイルの一覧を delay のあとに裏で読む。続けて呼ばれたら前の分を捨て、最後の呼び出しから数え直す
    /// （外したフォルダの変化が続くとき、重い walk_files を重ねて走らせないため）。
    fn load_file_list(&mut self, project: ProjectId, delay: Duration, cx: &mut Context<Self>) {
        let Some(fs) = self.live_fs(project) else { return };
        self.tree.file_list_task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let result = cx.background_spawn(async move { fs.walk_files() }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(files) => {
                        let query = this.tree.filter.as_ref().map(|f| f.read(cx).value().to_string()).unwrap_or_default();
                        this.tree.models.entry(project).or_default().set_filter(&query, Some(&files));
                    }
                    Err(e) => eprintln!("ファイルの一覧を読めない: {e}"),
                }
                this.tree.file_list_task = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// 監視の通知。読んだことのあるフォルダの中が変わったら読み直す。
    pub(crate) fn on_tree_changed(&mut self, project: ProjectId, batch: &WatchBatch, cx: &mut Context<Self>) {
        let Some(model) = self.tree.models.get_mut(&project) else { return };
        // 外す規則が変わったら、読んだフォルダをすべて読み直して印を直す。
        let dirs = if batch.rules { model.invalidate_all() } else { model.invalidate(&batch.changed) };
        let refetch = model.filtering();
        for dir in dirs {
            self.load_tree_dir(project, dir, cx);
        }
        if refetch {
            self.load_file_list(project, FILE_LIST_DELAY, cx);
        }
    }

    fn click_row(&mut self, row: TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        self.tree.selected = Some(row.rel.clone());
        if row.is_dir {
            let model = self.tree.models.entry(project).or_default();
            if model.filtering() || model.changed_only() {
                return;
            }
            if model.toggle(&row.rel) && model.needs_load(&row.rel) {
                self.load_tree_dir(project, row.rel, cx);
            }
            cx.notify();
        } else {
            self.open_tree_file(project, row.rel, window, cx);
        }
    }

    /// 名前の入力欄の行（計画8b）。通常の行と同じ字下げと高さで、アイコンと枠付きの入力欄を並べる。
    fn render_tree_edit_row(&self, ix: usize, depth: usize, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::tree_menu::EditKind;
        let Some(e) = self.tree.edit.as_ref() else { return div().into_any_element() };
        let folder = |expanded| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.))
                .flex_shrink_0()
                .child(icon(IconName::ChevronRight, theme.fg_dim))
                .child(file_icon(icon_for_folder(expanded)))
                .into_any_element()
        };
        let lead = match e.kind {
            EditKind::NewFolder => folder(false),
            EditKind::Rename if e.is_dir => folder(true),
            EditKind::NewFile | EditKind::Rename => {
                let name = e.input.read(cx).value().to_string();
                div().pl(px(16.)).flex_shrink_0().child(file_icon(icon_for_file(&name))).into_any_element()
            }
        };
        div()
            .id(ElementId::Name(format!("tree-row-{ix}").into()))
            .h(ROW_H)
            .w_full()
            .pl(px(7. + INDENT * depth as f32))
            .pr(px(4.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .rounded(size::RADIUS_SM)
            .border_1()
            .border_color(theme.accent)
            .on_action(cx.listener(|this, _: &gpui_kit::base::input::Escape, window, cx| this.cancel_tree_edit(window, cx)))
            .child(lead)
            .child(div().flex_1().min_w_0().child(Input::new(&e.input).appearance(false).h(ROW_H - px(4.))))
            .into_any_element()
    }

    /// 描画のたびに呼ぶ。ツリーの出入りの動きを始める。
    pub(crate) fn observe_tree(&mut self, now: Instant) {
        use crate::theme::motion::{PANEL_IN, PANEL_OUT};
        let reduced = self.reduced_motion;
        if self.tree.open && !self.tree_presence.visible() {
            self.tree_presence.show(PANEL_IN, reduced, now);
        } else if !self.tree.open && self.tree_presence.visible() {
            self.tree_presence.hide(PANEL_OUT, reduced, now);
        }
    }

    pub(crate) fn render_tree(&mut self, cx: &mut Context<Self>, now: Instant) -> Option<Stateful<Div>> {
        // 閉じたあとも、消える動きの間は描き続ける。
        if !self.tree.open && self.tree_presence.is_gone(now) {
            return None;
        }
            let project = self.workspace.active_project()?.id;
        let theme = *cx.global::<Theme>();
        let status = self.live.get(&project).and_then(|l| l.status.clone());
        let model = self.tree.models.entry(project).or_default();
        let rows = model.rows(status.as_ref());
        let changed_only = model.changed_only();
        self.tree.rows = rows_with_edit(rows, self.tree.edit.as_ref().map(|e| (&e.kind, &e.target)));
        let count = self.tree.rows.len();
        if std::mem::take(&mut self.tree.scroll_to_edit)
            && let Some(ix) = self.tree.rows.iter().position(|s| matches!(s, RowSlot::Edit { .. }))
        {
            self.tree.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        if self.tree.scroll_to_selected
            && let Some((ix, _)) = crate::tree_nav::selected_row(&self.tree.rows, self.tree.selected.as_ref())
        {
            self.tree.scroll_to_selected = false;
            self.tree.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        let measure = {
            let view = cx.entity().downgrade();
            let focus = self.tree.focus.clone();
            let filter = self.tree.filter.clone();
            canvas(
                move |bounds, _, cx| {
                    view.update(cx, |this, _| this.tree.list_bounds = bounds).ok();
                },
                // 本体にフォーカスがある間は、絞り込み欄を文字の入力の受け手にする。打った文字も IME の変換も
                // AltGr の文字も、そのまま欄に入る（入った時点で欄へフォーカスを移す）。受け手が無いと、GPUI は
                // IME を窓から外してしまう。
                move |bounds, _, window, cx| {
                    if let Some(filter) = filter {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, filter), cx);
                    }
                },
            )
            .absolute()
            .inset_0()
        };

        let toggle_changed = div()
            .id("tree-changed-only")
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .rounded(size::RADIUS_SM)
            .when(changed_only, |d| d.bg(theme.bg_hover))
            .hover(|s| s.bg(theme.bg_hover))
            .child(icon(IconName::FileDiff, if changed_only { theme.fg_default } else { theme.fg_dim }))
            .on_click(cx.listener(move |this, _, _, cx| {
                let model = this.tree.models.entry(project).or_default();
                model.set_changed_only(!model.changed_only());
                cx.notify();
            }));
        let filter = self.tree.filter.clone().map(|state| {
            Input::new(&state).appearance(false).prefix(icon(IconName::Search, theme.fg_dim)).cleanable(true)
        });
        let list = uniform_list(
            "tree-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let theme = *cx.global::<Theme>();
                let selected = this.tree.selected.clone();
                let root = this.workspace.active_project().map(|p| p.root.clone());
                this.tree.rows[range.start.min(this.tree.rows.len())..range.end.min(this.tree.rows.len())]
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(i, slot)| {
                        let row = match slot {
                            RowSlot::Entry(row) => row,
                            RowSlot::Edit { depth, .. } => return this.render_tree_edit_row(range.start + i, depth, &theme, cx),
                        };
                        let is_selected = selected.as_ref() == Some(&row.rel);
                        let chevron = if row.expanded { IconName::ChevronDown } else { IconName::ChevronRight };
                        // フォルダは開閉の矢印とフォルダのアイコン、ファイルは拡張子のアイコン（設計書 §5.5）。
                        let lead = if row.is_dir {
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(2.))
                                .flex_shrink_0()
                                .child(icon(chevron, theme.fg_dim))
                                .child(file_icon(icon_for_folder(row.expanded)))
                                .into_any_element()
                        } else {
                            // ファイルの行は、フォルダの矢印の分だけ右に寄せ、名前の頭を揃える。
                            div().pl(px(16.)).flex_shrink_0().child(file_icon(icon_for_file(&row.name))).into_any_element()
                        };
                        let deleted = row.mark == Some(fude_fs::git::GitMark::Deleted);
                        let mark = row.mark.map(|m| {
                            let (letter, color) = mark_style(m, &theme);
                            div().w(px(12.)).flex_shrink_0().font(mono_font()).text_size(px(11.)).text_color(color).child(letter)
                        });
                        let clicked = row.clone();
                        let menu_row = row.clone();
                        // ターミナルや Markdown へ落とせるよう、行をドラッグで運ぶ。
                        let dragged = root.as_ref().map(|root| crate::drag::DraggedEntry {
                            rel: row.rel.as_str().to_string(),
                            is_dir: row.is_dir,
                            location: crate::drag::entry_location(root, row.rel.as_str()),
                        });
                        div()
                            .id(ElementId::Name(format!("tree-row-{}", range.start + i).into()))
                            .when_some(dragged, |d, entry| {
                                d.on_drag(entry, |entry: &crate::drag::DraggedEntry, _, _, cx| cx.new(|_| entry.clone()))
                            })
                            .h(ROW_H)
                            .w_full()
                            .pl(px(8. + INDENT * row.depth as f32))
                            .pr(px(8.))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .rounded(size::RADIUS_SM)
                            .when(is_selected, |d| d.bg(theme.bg_hover))
                            .hover(|s| s.bg(theme.bg_hover))
                            .opacity(if deleted { 0.5 } else { 1.0 })
                            // git で外したもの（計画8d）は文字を薄くする（消した行の不透明度とは別）。
                            .when(row.ignored, |d| d.text_color(theme.fg_dim))
                            .child(lead)
                            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(row.name.clone()))
                            .children(mark)
                            .on_click(cx.listener(move |this, _, window, cx| this.click_row(clicked.clone(), window, cx)))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.open_tree_menu(Some(menu_row.clone()), ev.position, window, cx);
                                }),
                            )
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.tree.scroll)
        .flex_1();

        Some(
            div()
                .id("tree")
                .track_focus(&self.tree.focus)
                .key_context("Tree")
                .on_action(cx.listener(|this, _: &CloseTree, window, cx| this.close_tree(window, cx)))
                .on_action(cx.listener(|this, _: &TreeUndo, window, cx| this.undo_tree_op(window, cx)))
                .on_action(cx.listener(|this, _: &TreeRedo, window, cx| this.redo_tree_op(window, cx)))
                .on_action(cx.listener(|this, _: &TreeUp, _, cx| this.tree_nav(crate::tree_nav::NavKey::Up, cx)))
                .on_action(cx.listener(|this, _: &TreeDown, _, cx| this.tree_nav(crate::tree_nav::NavKey::Down, cx)))
                .on_action(cx.listener(|this, _: &TreeHome, _, cx| this.tree_nav(crate::tree_nav::NavKey::Home, cx)))
                .on_action(cx.listener(|this, _: &TreeEnd, _, cx| this.tree_nav(crate::tree_nav::NavKey::End, cx)))
                .on_action(cx.listener(|this, _: &TreePageUp, _, cx| this.tree_nav(crate::tree_nav::NavKey::PageUp, cx)))
                .on_action(cx.listener(|this, _: &TreePageDown, _, cx| this.tree_nav(crate::tree_nav::NavKey::PageDown, cx)))
                .on_action(cx.listener(|this, _: &TreeLeft, _, cx| this.tree_horizontal(false, cx)))
                .on_action(cx.listener(|this, _: &TreeRight, _, cx| this.tree_horizontal(true, cx)))
                .on_action(cx.listener(|this, _: &TreeOpen, window, cx| this.tree_open_selected(window, cx)))
                .on_action(cx.listener(|this, _: &TreeRename, window, cx| {
                    this.tree_action_on_selected(crate::tree_menu::TreeAction::Rename, window, cx)
                }))
                .on_action(cx.listener(|this, _: &TreeDelete, window, cx| {
                    this.tree_action_on_selected(crate::tree_menu::TreeAction::Delete, window, cx)
                }))
                .on_action(cx.listener(|this, _: &TreeMenu, window, cx| this.open_tree_menu_by_key(window, cx)))
                .on_action(cx.listener(|this, _: &TreeFilterDown, window, cx| this.leave_tree_filter(false, window, cx)))
                .on_action(cx.listener(|this, _: &TreeFilterCancel, window, cx| this.leave_tree_filter(true, window, cx)))
                // 絞り込み欄は、消すものが無いときに Esc を親へ回す。
                .on_action(cx.listener(|this, _: &gpui_kit::base::input::Escape, window, cx| this.close_tree(window, cx)))
                // 重ねて出したとき、下のターミナルにクリックを渡さない。
                .occlude()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(6.))
                .text_size(px(13.))
                .text_color(theme.fg_default)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.))
                        .children(filter.map(|f| div().key_context("TreeFilter").flex_1().child(f)))
                        .child(toggle_changed),
                )
                .child(
                    div()
                        .id("tree-body")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, ev: &MouseDownEvent, window, cx| this.open_tree_menu(None, ev.position, window, cx)),
                        )
                        .child(measure)
                        .child(list),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{CloseFocus, focus_after_close};

    #[test]
    fn closing_goes_to_editor_after_opening_from_tree() {
        // ツリーからファイルを開いていて、エディタがあれば、閉じたらエディタへ。
        assert_eq!(focus_after_close(true, true), CloseFocus::Editor);
        // それ以外は今までどおり（開く前の場所へ戻す）。
        assert_eq!(focus_after_close(true, false), CloseFocus::Restore);
        assert_eq!(focus_after_close(false, true), CloseFocus::Restore);
    }
}
