//! コマンドパレット（設計書 §5.9）。⌘K でコマンド、プロジェクト、ターミナル、ファイルを横断し、
//! ⌘P はファイル、⌘⇧F は全文検索に絞って開く。候補の組み立ては build_items（純粋な関数）が行う。

use std::sync::Arc;
use std::time::Instant;

use fude_core::fuzzy::rank;
use fude_core::keymap::Platform;
use fude_core::motion::Fade;
use fude_core::palette::{PaletteCommand, Scope, key_hint, parse_input};
use fude_core::workspace::{ProjectId, ProjectRoot, TabId};
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::popup::place;
use crate::theme::motion::{PALETTE_IN, PALETTE_OUT, REDUCED, ease_out_quint};
use crate::theme::{Theme, popup_shadow, size, ui_font};
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

gpui_kit::actions!(palette, [PaletteUp, PaletteDown, PaletteCancel, ToggleCase, ToggleWord, ToggleRegex]);

/// パレットの入力欄の文脈。gpui-kit の入力欄の上下キーと Esc より優先させる（同じ深さで後から登録したものが勝つ）。
const PALETTE_INPUT: &str = "Palette > Input";

pub fn bind_palette_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", PaletteUp, Some(PALETTE_INPUT)),
        KeyBinding::new("down", PaletteDown, Some(PALETTE_INPUT)),
        KeyBinding::new("escape", PaletteCancel, Some(PALETTE_INPUT)),
        // 全体検索の切り替え（VS Code と同じ Alt+C / Alt+W / Alt+R）。
        KeyBinding::new("alt-c", ToggleCase, Some(PALETTE_INPUT)),
        KeyBinding::new("alt-w", ToggleWord, Some(PALETTE_INPUT)),
        KeyBinding::new("alt-r", ToggleRegex, Some(PALETTE_INPUT)),
    ]);
}

pub(crate) const GROUP_COMMANDS: &str = "コマンド";
pub(crate) const GROUP_PROJECTS: &str = "プロジェクト";
pub(crate) const GROUP_TERMINALS: &str = "ターミナル";
pub(crate) const GROUP_FILES: &str = "ファイル";
pub(crate) const GROUP_SEARCH: &str = "全体検索";

/// 一覧に一度に見せる行の数。
const VISIBLE_ROWS: usize = 12;

/// 全体検索の切り替え（大文字小文字の区別、単語単位、正規表現）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SearchFlags {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// 選んだときにすること。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaletteTarget {
    Command(PaletteCommand),
    Project(ProjectRoot),
    Terminal(ProjectId, TabId),
    File(ProjectId, String),
    Search(String),
}

/// 一覧の 1 行。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaletteItem {
    pub target: PaletteTarget,
    pub label: String,
    /// 薄く添える文字（プロジェクトの場所、ファイルの親フォルダなど）。
    pub detail: Option<String>,
    /// 右端のキーの表示。
    pub hint: Option<String>,
    /// label の中の一致した文字（char）の位置。
    pub positions: Vec<u32>,
    pub group: &'static str,
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectSource {
    pub root: ProjectRoot,
    pub name: String,
    /// 場所の表示。
    pub place: String,
    pub open: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct TerminalSource {
    pub project: ProjectId,
    pub tab: TabId,
    pub title: String,
    pub project_name: String,
}

/// 候補の材料。
#[derive(Clone, Debug, Default)]
pub(crate) struct PaletteSources {
    pub projects: Vec<ProjectSource>,
    pub terminals: Vec<TerminalSource>,
    /// 選択中のプロジェクトのファイル（gitignore を尊重した一覧）。
    pub files: Option<(ProjectId, Arc<Vec<String>>)>,
    /// プロジェクトを開いている（ホーム画面ではない）。
    pub has_project: bool,
}

fn command_items(query: &str, sources: &PaletteSources, platform: Platform) -> Vec<PaletteItem> {
    let commands: Vec<PaletteCommand> =
        PaletteCommand::ALL.into_iter().filter(|c| sources.has_project || !c.needs_project()).collect();
    // 表示名と英字の別名を並べた文字列で探し、一致の印は表示名の中の分だけを残す。
    let texts: Vec<String> = commands.iter().map(|c| format!("{} {}", c.label(), c.keywords())).collect();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    rank(query, &texts, false, usize::MAX)
        .into_iter()
        .map(|r| {
            let command = commands[r.index];
            let label_chars = command.label().chars().count() as u32;
            PaletteItem {
                target: PaletteTarget::Command(command),
                label: command.label().into(),
                detail: None,
                hint: command.keymap_command().and_then(|c| key_hint(c, platform)),
                positions: r.positions.into_iter().filter(|&p| p < label_chars).collect(),
                group: GROUP_COMMANDS,
            }
        })
        .collect()
}

fn project_items(query: &str, sources: &PaletteSources) -> Vec<PaletteItem> {
    let labels: Vec<&str> = sources.projects.iter().map(|p| p.name.as_str()).collect();
    rank(query, &labels, false, usize::MAX)
        .into_iter()
        .map(|r| {
            let p = &sources.projects[r.index];
            PaletteItem {
                target: PaletteTarget::Project(p.root.clone()),
                label: p.name.clone(),
                detail: Some(p.place.clone()),
                hint: p.open.then(|| "開いている".to_string()),
                positions: r.positions,
                group: GROUP_PROJECTS,
            }
        })
        .collect()
}

fn terminal_items(query: &str, sources: &PaletteSources) -> Vec<PaletteItem> {
    let labels: Vec<&str> = sources.terminals.iter().map(|t| t.title.as_str()).collect();
    rank(query, &labels, false, usize::MAX)
        .into_iter()
        .map(|r| {
            let t = &sources.terminals[r.index];
            PaletteItem {
                target: PaletteTarget::Terminal(t.project, t.tab),
                label: t.title.clone(),
                detail: Some(t.project_name.clone()),
                hint: None,
                positions: r.positions,
                group: GROUP_TERMINALS,
            }
        })
        .collect()
}

fn file_items(query: &str, sources: &PaletteSources, limit: usize) -> Vec<PaletteItem> {
    let Some((project, files)) = &sources.files else { return Vec::new() };
    let paths: Vec<&str> = files.iter().map(String::as_str).collect();
    rank(query, &paths, true, limit)
        .into_iter()
        .map(|r| {
            let rel = paths[r.index];
            let (dir, name) = match rel.rsplit_once('/') {
                Some((dir, name)) => (Some(dir.to_string()), name),
                None => (None, rel),
            };
            // あいまい検索はパス全体で行うので、ファイル名の中の位置に直す。
            let start = (rel.chars().count() - name.chars().count()) as u32;
            PaletteItem {
                target: PaletteTarget::File(*project, rel.to_string()),
                label: name.to_string(),
                detail: dir,
                hint: None,
                positions: r.positions.into_iter().filter(|&p| p >= start).map(|p| p - start).collect(),
                group: GROUP_FILES,
            }
        })
        .collect()
}

/// ⌘P で出すファイルの数の上限。
pub(crate) const FILES_LIMIT: usize = 200;
/// ⌘K で文字を打ったときに出すファイルの数の上限。
pub(crate) const FILES_IN_ALL: usize = 30;

/// 範囲と文字列と候補の材料から一覧を作る。
/// Enter で選ぶ行。一覧が今の入力に追いついていなければ（stale）、今の入力で作り直した先頭を選ぶ。
pub(crate) fn confirm_choice(
    items: &[PaletteItem],
    selected: usize,
    stale: bool,
    rebuild: impl FnOnce() -> Vec<PaletteItem>,
) -> Option<PaletteItem> {
    if stale {
        return rebuild().into_iter().next();
    }
    items.get(selected).cloned()
}

pub(crate) fn build_items(scope: Scope, query: &str, sources: &PaletteSources, platform: Platform) -> Vec<PaletteItem> {
    match scope {
        Scope::Commands => command_items(query, sources, platform),
        Scope::Projects => project_items(query, sources),
        Scope::Files => file_items(query, sources, FILES_LIMIT),
        Scope::Search if query.is_empty() || !sources.has_project => Vec::new(),
        Scope::Search => vec![PaletteItem {
            target: PaletteTarget::Search(query.to_string()),
            label: format!("「{query}」を全体検索"),
            detail: None,
            hint: Some("Enter".into()),
            positions: Vec::new(),
            group: GROUP_SEARCH,
        }],
        Scope::All => {
            let mut items = command_items(query, sources, platform);
            items.extend(project_items(query, sources));
            items.extend(terminal_items(query, sources));
            if !query.is_empty() {
                items.extend(file_items(query, sources, FILES_IN_ALL));
            }
            items
        }
    }
}

/// 開いているパレットの状態。消える動きの途中も持ち続ける。
pub(crate) struct PaletteState {
    pub input: Entity<InputState>,
    /// ⌘K は All、⌘P は Files、⌘⇧F は Search（入力欄に "#" を入れて開く）。
    pub base: Scope,
    pub selected: usize,
    pub fade: Fade,
    pub closing: bool,
    /// 一覧の世代。入力のたびに 1 増やし、裏で作った一覧が古ければ捨てる。
    pub generation: u64,
    /// items を作った世代。generation と違えば、一覧はまだ今の入力に追いついていない。
    pub applied: u64,
    pub items: Vec<PaletteItem>,
    pub search_flags: SearchFlags,
    pub error: Option<String>,
    /// 開く前にフォーカスがあったところ。閉じたら戻す。
    return_focus: Option<FocusHandle>,
    scroll: ScrollHandle,
    _sub: Subscription,
}

fn placeholder(base: Scope) -> &'static str {
    match base {
        Scope::Files => "ファイル名で探す",
        Scope::Search => "全体検索（# のあとに文字を入れて Enter）",
        _ => "コマンド、プロジェクト、ターミナル、ファイル（> コマンド、@ プロジェクト、# 全体検索）",
    }
}

/// 全体検索の切り替えのボタン（オンのときは枠と明るい文字）。
fn palette_toggle(
    id: &'static str,
    label: &'static str,
    on: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
    change: fn(&mut SearchFlags),
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(22.))
        .px(px(6.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .font(crate::theme::mono_font())
        .text_size(px(12.))
        .text_color(if on { theme.fg_default } else { theme.fg_dim })
        .when(on, |d| d.bg(theme.bg_hover).border_1().border_color(theme.accent))
        .hover(|s| s.bg(theme.bg_hover))
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.toggle_search_flag(change, cx);
            // クリックで外れた入力欄のフォーカスを戻し、続けて打てるようにする。
            if let Some(p) = this.palette.as_ref() {
                p.input.update(cx, |s, cx| s.focus(window, cx));
            }
        }))
}

/// 一覧の中の、selected の行の子の番号（グループの見出しも 1 行に数える）。
fn row_index(items: &[PaletteItem], selected: usize) -> usize {
    let headers = items[..=selected.min(items.len().saturating_sub(1))]
        .iter()
        .enumerate()
        .filter(|(i, item)| *i == 0 || items[i - 1].group != item.group)
        .count();
    selected + headers
}

/// char の位置を、label のバイトの範囲（1 文字ずつ）にする。
fn char_ranges(label: &str, positions: &[u32]) -> Vec<std::ops::Range<usize>> {
    label
        .char_indices()
        .enumerate()
        .filter(|(i, _)| positions.contains(&(*i as u32)))
        .map(|(_, (b, c))| b..b + c.len_utf8())
        .collect()
}

impl WorkspaceView {
    /// パレットを開く。開いていて同じ範囲ならば閉じる（同じキーをもう一度押したとき）。
    pub(crate) fn open_palette(&mut self, base: Scope, initial: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.palette.as_mut().filter(|p| !p.closing) {
            if p.base == base {
                self.close_palette(window, cx);
                return;
            }
            p.base = base;
            let input = p.input.clone();
            input.update(cx, |s, cx| s.set_value(initial.to_string(), window, cx));
            self.refresh_palette(cx);
            return;
        }
        let return_focus = window.focused(cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder(base)));
        input.update(cx, |s, cx| {
            s.set_value(initial.to_string(), window, cx);
            s.focus(window, cx);
        });
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => this.refresh_palette(cx),
            InputEvent::PressEnter { .. } => this.confirm_palette(window, cx),
            _ => {}
        });
        let mut fade = Fade::settled(0.0);
        let duration = if self.reduced_motion { REDUCED } else { PALETTE_IN };
        fade.animate_to(1.0, duration, ease_out_quint, Instant::now());
        if let Some(project) = self.workspace.active_project().map(|p| p.id) {
            self.refresh_files(project, cx);
        }
        self.palette = Some(PaletteState {
            input,
            base,
            selected: 0,
            fade,
            closing: false,
            generation: 0,
            applied: 0,
            items: Vec::new(),
            search_flags: SearchFlags::default(),
            error: None,
            return_focus,
            scroll: ScrollHandle::new(),
            _sub: sub,
        });
        self.refresh_palette(cx);
    }

    /// 消える動きを始め、開く前のフォーカスへ戻す。
    pub(crate) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reduced = self.reduced_motion;
        let Some(p) = self.palette.as_mut().filter(|p| !p.closing) else { return };
        p.closing = true;
        let duration = if reduced { REDUCED } else { PALETTE_OUT };
        p.fade.animate_to(0.0, duration, ease_out_quint, Instant::now());
        match p.return_focus.take() {
            Some(focus) => window.focus(&focus, cx),
            None => self.restore_focus(window, cx),
        }
        cx.notify();
    }

    /// 候補の材料を今の状態から集める。
    pub(crate) fn palette_sources(&self) -> PaletteSources {
        let projects = self.workspace.projects();
        let mut sources: Vec<ProjectSource> = projects
            .iter()
            .map(|p| ProjectSource { root: p.root.clone(), name: p.name.clone(), place: p.root.copy_text(), open: true })
            .collect();
        for entry in self.recent.top(20) {
            if !projects.iter().any(|p| p.root.same_place(&entry.root)) {
                sources.push(ProjectSource {
                    root: entry.root.clone(),
                    name: entry.root.name(),
                    place: entry.root.copy_text(),
                    open: false,
                });
            }
        }
        let terminals = projects
            .iter()
            .flat_map(|p| {
                p.tabs.iter().map(|t| TerminalSource { project: p.id, tab: t.id, title: t.title(), project_name: p.name.clone() })
            })
            .collect();
        let active = self.workspace.active_project().map(|p| p.id);
        let files = active.and_then(|id| Some((id, self.live.get(&id)?.files.clone()?)));
        PaletteSources { projects: sources, terminals, files, has_project: active.is_some() }
    }

    /// 入力から一覧を作り直す。ファイルが多いと時間がかかるので裏で作り、古い世代の結果は捨てる。
    /// 作り終えるまでは前の一覧を出したままにする。
    pub(crate) fn refresh_palette(&mut self, cx: &mut Context<Self>) {
        let sources = self.palette_sources();
        let Some(p) = self.palette.as_mut() else { return };
        let text = p.input.read(cx).value().to_string();
        let base = p.base;
        p.generation += 1;
        p.error = None;
        let generation = p.generation;
        let task = cx.background_spawn(async move {
            let (scope, query) = parse_input(&text, base);
            build_items(scope, query, &sources, Platform::current())
        });
        cx.spawn(async move |this, cx| {
            let items = task.await;
            this.update(cx, |this, cx| {
                let Some(p) = this.palette.as_mut().filter(|p| p.generation == generation) else { return };
                p.items = items;
                p.applied = generation;
                p.selected = 0;
                p.scroll.scroll_to_item(0);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn toggle_search_flag(&mut self, change: fn(&mut SearchFlags), cx: &mut Context<Self>) {
        let Some(p) = self.palette.as_mut() else { return };
        change(&mut p.search_flags);
        p.error = None;
        cx.notify();
    }

    fn move_palette_selection(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(p) = self.palette.as_mut() else { return };
        let n = p.items.len() as i32;
        if n == 0 {
            return;
        }
        p.selected = (p.selected as i32 + delta).rem_euclid(n) as usize;
        p.scroll.scroll_to_item(row_index(&p.items, p.selected));
        cx.notify();
    }

    /// 選択中の行を実行する。
    fn confirm_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.palette.as_ref().filter(|p| !p.closing) else { return };
        // 打ってすぐの Enter（裏で一覧を作っている途中）でも、今の入力どおりに動く。
        let stale = p.applied != p.generation;
        let (text, base) = (p.input.read(cx).value().to_string(), p.base);
        let rebuild = || {
            let sources = self.palette_sources();
            let (scope, query) = parse_input(&text, base);
            build_items(scope, query, &sources, Platform::current())
        };
        let Some(item) = confirm_choice(&p.items, p.selected, stale, rebuild) else { return };
        // 正しくない検索の条件なら理由を出し、閉じない。
        if let (PaletteTarget::Search(pattern), Some(p)) = (&item.target, self.palette.as_mut())
            && let Err(e) = fude_fs::search::build_pattern(&crate::search_view::make_query(pattern, p.search_flags))
        {
            p.error = Some(e);
            cx.notify();
            return;
        }
        self.close_palette(window, cx);
        self.run_palette_item(item, window, cx);
    }

    fn run_palette_item(&mut self, item: PaletteItem, window: &mut Window, cx: &mut Context<Self>) {
        match item.target {
            PaletteTarget::Command(command) => self.run_palette_command(command, window, cx),
            PaletteTarget::Project(root) => {
                let open = self.workspace.projects().iter().find(|p| p.root.same_place(&root)).map(|p| p.id);
                match open {
                    Some(id) => {
                        self.workspace.activate_project(id);
                        self.refresh_active_branch(cx);
                        self.refocus_pane();
                    }
                    None => self.open_project(root, None, cx),
                }
            }
            PaletteTarget::Terminal(project, tab) => {
                self.workspace.activate_project(project);
                self.workspace.activate_tab(tab);
                self.refresh_active_branch(cx);
                self.refocus_pane();
            }
            PaletteTarget::File(project, rel) => self.open_in_editor(project, rel, None, None, window, cx),
            PaletteTarget::Search(pattern) => {
                let flags = self.palette.as_ref().map(|p| p.search_flags).unwrap_or_default();
                self.start_search(crate::search_view::make_query(&pattern, flags), window, cx);
            }
        }
        cx.notify();
    }

    /// コマンドを実行する。キーと同じアクションを、戻したフォーカスの位置へ送る。
    fn run_palette_command(&mut self, command: PaletteCommand, window: &mut Window, cx: &mut Context<Self>) {
        use crate::workspace_view::{CloseItem, FindInTerminal, NewTab, NextTab, PrevTab, SplitTab};
        let action: Box<dyn Action> = match command {
            PaletteCommand::NewTerminal => Box::new(NewTab),
            PaletteCommand::SplitTerminal => Box::new(SplitTab),
            PaletteCommand::ToggleTree => Box::new(crate::tree_view::ToggleTree),
            PaletteCommand::Close => Box::new(CloseItem),
            PaletteCommand::NextTab => Box::new(NextTab),
            PaletteCommand::PrevTab => Box::new(PrevTab),
            PaletteCommand::FindInTerminal => Box::new(FindInTerminal),
            PaletteCommand::ToggleRouteMap => Box::new(crate::workspace_view::ToggleRouteMap),
            PaletteCommand::Save => {
                // 保存はファイルの中で効くので、選択中のファイルへフォーカスを移してから送る。
                let Some(view) = self.active_editor_view() else { return };
                window.focus(&view.read(cx).focus_handle(), cx);
                Box::new(crate::file_view::SaveFile)
            }
            PaletteCommand::OpenFolder => return self.prompt_open_project(cx),
            PaletteCommand::GoToFile => return self.open_palette(Scope::Files, "", window, cx),
            PaletteCommand::FindInFiles => return self.open_find_in_files(window, cx),
            PaletteCommand::LaunchClaude => return self.launch_claude(cx),
            PaletteCommand::CopyHookConfig => return self.copy_hook_config(window, cx),
            // 中身は計画8 の後のタスクで入れる。
            PaletteCommand::ToggleMarkdownMode => {
                let Some(view) = self.active_editor_view() else { return };
                window.focus(&view.read(cx).focus_handle(), cx);
                Box::new(crate::file_view::ToggleMarkdownMode)
            }
            PaletteCommand::TogglePin => {
                // 選択中のタブが Markdown ならそれを切り替える。そうでなければピンを外す。
                let Some(p) = self.workspace.active_project() else { return };
                let project = p.id;
                let rel = p.editor.active().map(|i| i.rel.clone()).filter(|r| fude_core::markdown::is_markdown(r));
                return self.toggle_pin(project, rel, cx);
            }
            PaletteCommand::ToggleVerticalTabs => return self.toggle_vertical_tabs(cx),
        };
        window.dispatch_action(action, cx);
    }

    pub(crate) fn render_palette(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Option<Deferred> {
        let p = self.palette.as_ref()?;
        if p.closing && !p.fade.is_animating(now) {
            self.palette = None;
            return None;
        }
        if p.fade.is_animating(now) {
            window.request_animation_frame();
        }
        let theme = *cx.global::<Theme>();
        let value = p.fade.value(now);
        let viewport = window.viewport_size().width;
        let w = size::PALETTE_W.min(viewport - px(32.));
        // 消えるときは不透明度だけ（§6.3）。動きを減らす設定では動かさない。
        let shift = if self.reduced_motion || p.closing { px(0.) } else { size::PALETTE_SHIFT * (1.0 - value) };
        let position = point((viewport - w) / 2., size::PALETTE_TOP - shift);

        let mut rows: Vec<AnyElement> = Vec::new();
        let mut last_group = None;
        for (i, item) in p.items.iter().enumerate() {
            if last_group != Some(item.group) {
                last_group = Some(item.group);
                rows.push(
                    div()
                        .h(px(22.))
                        .px(px(10.))
                        .flex()
                        .items_end()
                        .text_size(px(11.))
                        .text_color(theme.fg_dim)
                        .child(item.group)
                        .into_any_element(),
                );
            }
            rows.push(self.render_palette_row(i, item, i == p.selected, &theme, cx));
        }
        let error = p.error.clone().map(|e| div().px(px(12.)).py(px(6.)).text_size(px(12.)).text_color(theme.danger).child(e));
        let list_h = size::PALETTE_ROW_H * VISIBLE_ROWS as f32;
        let input = Input::new(&p.input).appearance(false).prefix(icon(IconName::Search, theme.fg_dim));
        let text = p.input.read(cx).value().to_string();
        let toggles = (parse_input(&text, p.base).0 == Scope::Search).then(|| {
            let f = p.search_flags;
            div()
                .flex()
                .flex_row()
                .gap(px(2.))
                .flex_shrink_0()
                .child(palette_toggle("palette-case", "Aa", f.case_sensitive, &theme, cx, |f| f.case_sensitive = !f.case_sensitive))
                .child(palette_toggle("palette-word", "ab", f.whole_word, &theme, cx, |f| f.whole_word = !f.whole_word))
                .child(palette_toggle("palette-regex", ".*", f.regex, &theme, cx, |f| f.regex = !f.regex))
        });
        let body = div()
            .id("palette")
            .key_context("Palette")
            .occlude()
            .on_action(cx.listener(|this, _: &PaletteUp, _, cx| this.move_palette_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &PaletteDown, _, cx| this.move_palette_selection(1, cx)))
            .on_action(cx.listener(|this, _: &PaletteCancel, window, cx| this.close_palette(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleCase, _, cx| this.toggle_search_flag(|f| f.case_sensitive = !f.case_sensitive, cx)))
            .on_action(cx.listener(|this, _: &ToggleWord, _, cx| this.toggle_search_flag(|f| f.whole_word = !f.whole_word, cx)))
            .on_action(cx.listener(|this, _: &ToggleRegex, _, cx| this.toggle_search_flag(|f| f.regex = !f.regex, cx)))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_palette(window, cx)))
            .opacity(value)
            .w(w)
            .flex()
            .flex_col()
            .rounded(size::RADIUS_MD)
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border_subtle)
            .shadow(popup_shadow())
            .font(ui_font())
            .text_size(px(13.))
            .text_color(theme.fg_default)
            .child(div().h(px(40.)).px(px(10.)).flex().items_center().gap(px(6.)).child(div().flex_1().min_w_0().child(input)).children(toggles))
            .children(error)
            .when(!rows.is_empty(), |d| {
                d.child(div().h(px(1.)).bg(theme.border_subtle)).child(
                    div()
                        .id("palette-list")
                        .track_scroll(&p.scroll)
                        .max_h(list_h)
                        .overflow_y_scroll()
                        .p(px(4.))
                        .flex()
                        .flex_col()
                        .children(rows),
                )
            });
        Some(place(position, body))
    }

    fn render_palette_row(&self, i: usize, item: &PaletteItem, selected: bool, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let lead: AnyElement = match &item.target {
            PaletteTarget::Command(_) => icon(IconName::ChevronRight, theme.fg_dim).into_any_element(),
            PaletteTarget::Project(_) => icon(IconName::FolderOpen, theme.fg_dim).into_any_element(),
            PaletteTarget::Terminal(..) => icon(IconName::Terminal, theme.fg_dim).into_any_element(),
            PaletteTarget::Search(_) => icon(IconName::Search, theme.fg_dim).into_any_element(),
            PaletteTarget::File(_, rel) => {
                let name = rel.rsplit('/').next().unwrap_or(rel);
                crate::file_icons::file_icon(crate::file_icons::icon_for_file(name)).into_any_element()
            }
        };
        let strong = HighlightStyle { color: Some(theme.fg_default.into()), font_weight: Some(FontWeight::BOLD), ..Default::default() };
        let highlights = char_ranges(&item.label, &item.positions).into_iter().map(|r| (r, strong)).collect::<Vec<_>>();
        let label_color = if item.positions.is_empty() { theme.fg_default } else { theme.fg_dim };
        div()
            .id(ElementId::Name(format!("palette-row-{i}").into()))
            .h(size::PALETTE_ROW_H)
            .flex_shrink_0()
            .px(px(10.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .rounded(size::RADIUS_SM)
            .when(selected, |d| d.bg(theme.bg_hover))
            .child(lead)
            .child(
                div()
                    .text_color(label_color)
                    .whitespace_nowrap()
                    .child(StyledText::new(item.label.clone()).with_highlights(highlights)),
            )
            .children(item.detail.clone().map(|d| {
                div().flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().text_size(px(12.)).text_color(theme.fg_dim).child(d)
            }))
            .when(item.detail.is_none(), |d| d.child(div().flex_1()))
            .children(item.hint.clone().map(|h| div().flex_shrink_0().text_size(px(12.)).text_color(theme.fg_dim).child(h)))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if let Some(p) = this.palette.as_mut().filter(|p| p.selected != i) {
                    p.selected = i;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(p) = this.palette.as_mut() {
                    p.selected = i;
                }
                this.confirm_palette(window, cx);
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GROUP_COMMANDS, GROUP_FILES, GROUP_PROJECTS, GROUP_TERMINALS, PaletteSources, PaletteTarget, ProjectSource, TerminalSource,
        build_items, confirm_choice,
    };
    use fude_core::keymap::Platform;
    use fude_core::palette::{PaletteCommand, Scope};
    use fude_core::workspace::{ProjectRoot, Workspace};
    use std::sync::Arc;

    fn sources_with_one_project() -> PaletteSources {
        let mut ws = Workspace::default();
        let root = ProjectRoot::Local("/w/web".into());
        let id = ws.open_project(root.clone());
        let p = ws.active_project().unwrap();
        let tab = p.tabs[0].id;
        PaletteSources {
            projects: vec![ProjectSource { root, name: "web".into(), place: "/w/web".into(), open: true }],
            terminals: vec![TerminalSource { project: id, tab, title: "pwsh".into(), project_name: "web".into() }],
            files: None,
            has_project: true,
        }
    }

    #[test]
    fn all_scope_groups_in_order_without_files_when_empty() {
        let items = build_items(Scope::All, "", &sources_with_one_project(), Platform::Windows);
        let mut groups: Vec<&str> = Vec::new();
        for i in &items {
            if groups.last() != Some(&i.group) {
                groups.push(i.group);
            }
        }
        assert_eq!(groups, [GROUP_COMMANDS, GROUP_PROJECTS, GROUP_TERMINALS]);
    }

    #[test]
    fn enter_before_the_list_catches_up_uses_the_current_input() {
        // "#foo_ba" の一覧のまま、"#foo_bar" で Enter。今の入力から作り直したほうを選ぶ。
        let sources = sources_with_one_project();
        let old = build_items(Scope::Search, "foo_ba", &sources, Platform::Windows);
        let fresh = || build_items(Scope::Search, "foo_bar", &sources, Platform::Windows);
        assert_eq!(confirm_choice(&old, 0, true, fresh).unwrap().target, PaletteTarget::Search("foo_bar".into()));
        // 一覧が今の入力のものなら、選んでいる行を使う（作り直さない）。
        let got = confirm_choice(&old, 0, false, || unreachable!("作り直さない"));
        assert_eq!(got.unwrap().target, PaletteTarget::Search("foo_ba".into()));
    }

    #[test]
    fn commands_show_key_hints() {
        let items = build_items(Scope::Commands, "新しい", &sources_with_one_project(), Platform::Windows);
        assert_eq!(items[0].target, PaletteTarget::Command(PaletteCommand::NewTerminal));
        assert_eq!(items[0].hint.as_deref(), Some("Ctrl+Shift+T"));
    }

    #[test]
    fn commands_match_english_keywords() {
        // 表示名は日本語なので、英字で打っても見つかるよう別名でも探す。一致の印は表示名の中だけに付ける。
        let items = build_items(Scope::Commands, "split", &sources_with_one_project(), Platform::Windows);
        assert_eq!(items[0].target, PaletteTarget::Command(PaletteCommand::SplitTerminal));
        let label_chars = items[0].label.chars().count() as u32;
        assert!(items[0].positions.iter().all(|&p| p < label_chars));
        let items = build_items(Scope::Commands, "tab", &sources_with_one_project(), Platform::Windows);
        assert!(items.iter().any(|i| i.target == PaletteTarget::Command(PaletteCommand::NextTab)));
    }

    #[test]
    fn home_screen_hides_project_commands() {
        let items = build_items(Scope::Commands, "", &PaletteSources { has_project: false, ..Default::default() }, Platform::MacOs);
        let cmds: Vec<PaletteTarget> = items.into_iter().map(|i| i.target).collect();
        assert!(cmds.contains(&PaletteTarget::Command(PaletteCommand::OpenFolder)));
        assert!(!cmds.contains(&PaletteTarget::Command(PaletteCommand::NewTerminal)));
    }

    #[test]
    fn row_index_counts_group_headers() {
        let items = build_items(Scope::All, "", &sources_with_one_project(), Platform::Windows);
        assert_eq!(super::row_index(&items, 0), 1);
        // 最初のプロジェクトの行は、コマンドの見出しとプロジェクトの見出しの 2 つぶん後ろ。
        let first_project = items.iter().position(|i| i.group == GROUP_PROJECTS).unwrap();
        assert_eq!(super::row_index(&items, first_project), first_project + 2);
    }

    #[test]
    fn files_scope_limits_and_ranks() {
        let project = sources_with_one_project().terminals[0].project;
        let files: Vec<String> = (0..500).map(|i| format!("src/f{i}.rs")).chain(["src/main.rs".to_string()]).collect();
        let src = PaletteSources { files: Some((project, Arc::new(files))), has_project: true, ..Default::default() };
        let items = build_items(Scope::Files, "main", &src, Platform::Windows);
        assert_eq!(items[0].target, PaletteTarget::File(project, "src/main.rs".into()));
        assert_eq!(items[0].label, "main.rs");
        assert_eq!(items[0].detail.as_deref(), Some("src"));
        assert!(build_items(Scope::Files, "", &src, Platform::Windows).len() <= 200);
        // ⌘K で文字を打ったときのファイルは 30 件まで。
        assert!(build_items(Scope::All, "f", &src, Platform::Windows).iter().filter(|i| i.group == GROUP_FILES).count() <= 30);
    }

    #[test]
    fn file_positions_are_shifted_to_the_name() {
        // あいまい検索はパス全体で行い、表示はファイル名なので、名前の中の位置に直す（名前の外の一致は落とす）。
        let project = sources_with_one_project().terminals[0].project;
        let src = PaletteSources { files: Some((project, Arc::new(vec!["src/main.rs".into()]))), has_project: true, ..Default::default() };
        let items = build_items(Scope::Files, "main", &src, Platform::Windows);
        assert_eq!(items[0].positions, [0, 1, 2, 3]);
        let root_file = PaletteSources { files: Some((project, Arc::new(vec!["README.md".into()]))), has_project: true, ..Default::default() };
        assert_eq!(build_items(Scope::Files, "", &root_file, Platform::Windows)[0].detail, None);
    }

    #[test]
    fn search_scope_offers_one_item() {
        let items = build_items(Scope::Search, "foo", &sources_with_one_project(), Platform::MacOs);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target, PaletteTarget::Search("foo".into()));
        assert!(build_items(Scope::Search, "", &sources_with_one_project(), Platform::MacOs).is_empty());
    }
}
