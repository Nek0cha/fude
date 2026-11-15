//! 初回のチュートリアル（9c の設計書 `2026-11-15-tutorial-design.md`）。

use std::path::Path;
use std::time::Instant;

use fude_core::keymap::{Command, KeyHints, Platform, key_hints};
use fude_core::palette::{PaletteCommand, key_label};
use fude_core::tutorial::{DONE_TEXT, StartupTutorial, is_done, startup_tutorial};
use gpui_kit::*;

use crate::config_runtime::ConfigState;
use crate::hook_targets::HookBoard;
use crate::settings_sections::{AppearanceSection, HookSection, SectionMode, TextSection, chip, heading, small_button};
use crate::theme::Theme;

/// 「見た」の印を書く（親のフォルダが無ければ作る）。
pub(crate) fn write_mark(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, DONE_TEXT)
}

/// 起動時にチュートリアルを出すかを決める（§5.1）。9c より前から使っている人には、その場で印を付ける（失敗は無視）。
pub(crate) fn decide_startup(home_launch: bool, mark: Option<&Path>, recent_empty: bool) -> StartupTutorial {
    let done = is_done(mark.and_then(|p| std::fs::read_to_string(p).ok()).as_deref());
    let decision = startup_tutorial(home_launch, done, recent_empty);
    if decision == StartupTutorial::MarkDone
        && let Some(p) = mark
    {
        write_mark(p).ok();
    }
    decision
}

/// パレットの命令を実行する前に、チュートリアルを閉じるか（§5.6）。もう一度見る命令と設定（何もしない）は閉じない。
pub(crate) fn closes_tutorial(command: PaletteCommand) -> bool {
    !matches!(command, PaletteCommand::ShowTutorial | PaletteCommand::OpenSettings)
}

/// キーの命令で、チュートリアルを閉じるか（§5.6、§5.7）。パレットを開くだけでは画面は変わらないので閉じない。
/// Ctrl+, は何もしない。コピーと貼り付けはターミナルの中だけで効くので閉じない。
/// プロジェクトが無いときは、どの命令も画面を変えないので閉じない（2 ページ目で教えたキーを試しても消えない）。
pub(crate) fn key_command_closes(command: Command, has_project: bool) -> bool {
    has_project && !matches!(command, Command::OpenPalette | Command::OpenSettings | Command::Copy | Command::Paste)
}

/// チュートリアルを閉じたあとのフォーカスの行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeaveFocus {
    /// 命令が移したフォーカスをそのままにする。
    Keep,
    /// 選択中のターミナルへすぐに移す。
    Terminal,
    /// ワークスペース自身へ移す。
    Workspace,
}

/// owned はチュートリアルが閉じる直前にフォーカスを持っていたか、has_terminal は選択中のターミナルがあるか。
/// 命令がすでにフォーカスを移していればそのままにする。移すときは次の描画を待たずに移し、あとから開くパレットなどの
/// フォーカスを描画で上書きしないようにする。
pub(crate) fn leave_focus(owned: bool, has_terminal: bool) -> LeaveFocus {
    match (owned, has_terminal) {
        (false, _) => LeaveFocus::Keep,
        (true, true) => LeaveFocus::Terminal,
        (true, false) => LeaveFocus::Workspace,
    }
}

/// ページ（§5.2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Look,
    Basics,
    Claude,
    Ready,
}

const PAGES: [Page; 4] = [Page::Look, Page::Basics, Page::Claude, Page::Ready];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nav {
    Back,
    Next,
}

/// 隣のページ。最初のページで戻る、最後のページで進む（はじめる）ときは None。
pub(crate) fn step(page: Page, nav: Nav) -> Option<Page> {
    let i = PAGES.iter().position(|p| *p == page)?;
    match nav {
        Nav::Back => i.checked_sub(1).map(|j| PAGES[j]),
        Nav::Next => PAGES.get(i + 1).copied(),
    }
}

/// キーの表示を 1 つの句にする。両方あれば「外のキー（ターミナルの中では どこでものキー）」。
fn keys_phrase(h: &KeyHints) -> Option<String> {
    match (&h.outside_terminal, &h.anywhere) {
        (Some(o), Some(a)) if o != a => Some(format!("{o}（ターミナルの中では {a}）")),
        (Some(k), _) | (None, Some(k)) => Some(k.clone()),
        (None, None) => None,
    }
}

/// 2 ページ目のツリーの文。割り当ての無いキーは書かない。
pub(crate) fn tree_text(h: &KeyHints) -> String {
    match (&h.outside_terminal, &h.anywhere) {
        (Some(o), Some(a)) if o != a => format!("上のプロジェクトのピルをクリックするか、{o} を押します（ターミナルの中では {a}）。"),
        (Some(k), _) | (None, Some(k)) => format!("上のプロジェクトのピルをクリックするか、{k} を押します。"),
        (None, None) => "上のプロジェクトのピルをクリックします。".to_string(),
    }
}

/// チュートリアルが WorkspaceView に頼むこと。
pub(crate) enum TutorialEvent {
    /// 「はじめる」か「スキップ」。印と設定ファイルは受け取った側が書く。
    Finished,
}

impl EventEmitter<TutorialEvent> for TutorialView {}

pub(crate) struct TutorialView {
    focus: FocusHandle,
    scroll: ScrollHandle,
    page: Page,
    /// 3 ページ目の答え（保存しない。§5.2）。
    uses_claude: Option<bool>,
    /// 動く絵の時刻の起点。
    started: Instant,
    reduced_motion: bool,
    appearance: Entity<AppearanceSection>,
    text: Entity<TextSection>,
    hooks: Entity<HookSection>,
    _subs: Vec<Subscription>,
}

impl TutorialView {
    pub fn new(board: Entity<HookBoard>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = cx.new(|cx| AppearanceSection::new(SectionMode::FirstRun, window, cx));
        let text = cx.new(|cx| TextSection::new(SectionMode::FirstRun, window, cx));
        let hooks = cx.new(|cx| HookSection::new(board, SectionMode::FirstRun, cx));
        // キーの表示は今の割り当てから作るので、設定が変われば描き直す。
        let subs = vec![cx.observe_global::<ConfigState>(|_, cx| cx.notify())];
        TutorialView {
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            page: Page::Look,
            uses_claude: None,
            started: Instant::now(),
            reduced_motion: crate::reduced_motion::reduced_motion(),
            appearance,
            text,
            hooks,
            _subs: subs,
        }
    }

    /// 1 ページ目からやり直す（パレットの「チュートリアルをもう一度」）。
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Look;
        self.uses_claude = None;
        self.started = Instant::now();
        self.reduced_motion = crate::reduced_motion::reduced_motion();
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn go(&mut self, nav: Nav, cx: &mut Context<Self>) {
        match step(self.page, nav) {
            Some(page) => {
                self.page = page;
                self.started = Instant::now();
                self.scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }
            None if nav == Nav::Next => cx.emit(TutorialEvent::Finished),
            None => {}
        }
    }

    fn render_look(&self, hints: &Hints, theme: &Theme) -> Div {
        let lead = match &hints.settings {
            Some(k) => format!("最初に見た目を決めましょう。あとから {k} の設定でいつでも変えられます。"),
            None => "最初に見た目を決めましょう。あとから設定でいつでも変えられます。".to_string(),
        };
        div()
            .child(title("fude へようこそ", theme))
            .child(lead_text(lead, theme))
            .child(self.appearance.clone())
            .child(self.text.clone())
    }

    fn render_basics(&self, hints: &Hints, theme: &Theme) -> Div {
        let frame = (!self.reduced_motion).then(|| crate::tutorial_demo::demo_frame(self.started.elapsed()));
        let demo_keys = hints.tree.outside_terminal.clone().or_else(|| hints.tree.anywhere.clone()).unwrap_or_default();
        let open_file = match &hints.go_to_file {
            Some(k) => format!("ツリー、{k}、ターミナルに出たパスのクリックで開けます。開いたファイルは右のエディタ列にタブで並びます。"),
            None => "ツリーか、ターミナルに出たパスのクリックで開けます。開いたファイルは右のエディタ列にタブで並びます。".to_string(),
        };
        let palette = match &hints.palette {
            Some(k) => format!("{k} で開きます。ほとんどの操作はここから呼べます。"),
            None => "ほとんどの操作はここから呼べます。".to_string(),
        };
        let settings = match &hints.settings {
            Some(k) => format!("{k} で、見た目などをいつでも変えられます。"),
            None => "コマンドパレットの「設定を開く」から、見た目などをいつでも変えられます。".to_string(),
        };
        let tree = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(16.))
            .child(div().flex_1().min_w(px(220.)).child(body(tree_text(&hints.tree), theme)))
            .child(crate::tutorial_demo::tree_demo(frame, &demo_keys, theme));
        div()
            .child(title("基本の使い方", theme))
            .child(item(
                "フォルダを開く",
                body("ホーム画面の「フォルダを開く」か、左のレールの「＋」から開きます。フォルダはプロジェクトになり、ターミナルが 1 枚立ち上がります。", theme),
                theme,
            ))
            .child(item("ファイルツリー", tree, theme))
            .child(item("ファイルを開く", body(open_file, theme), theme))
            .child(item("コマンドパレット", body(palette, theme), theme))
            .child(item("設定", body(settings, theme), theme))
    }

    fn render_claude(&self, hints: &Hints, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let answer = |id: &'static str, label: &'static str, value: bool| {
            chip(id, label, self.uses_claude == Some(value), theme).on_click(cx.listener(move |this, _, _, cx| {
                this.uses_claude = Some(value);
                if !value {
                    this.go(Nav::Next, cx);
                }
                cx.notify();
            }))
        };
        let mut el = div()
            .child(title("Claude Code を使っていますか？", theme))
            .child(div().pt(px(8.)).flex().flex_row().gap(px(8.)).child(answer("claude-yes", "はい", true)).child(answer("claude-no", "いいえ", false)));
        if self.uses_claude == Some(true) {
            let route = match &hints.route_map {
                Some(k) => format!("プランのタスクを路線図に出します（{k}）。"),
                None => "プランのタスクを路線図に出します。".to_string(),
            };
            let send = match &hints.send_to_claude {
                Some(k) => format!("ファイルの中で {k} を押すと、そのファイルを Claude に渡せます。"),
                None => "ファイルの上の「Claude に渡す」で、そのファイルを Claude に渡せます。".to_string(),
            };
            el = el
                .child(div().pt(px(16.)).child(heading("fude でできること", theme)))
                .child(body("Claude Code が返事を待つと、レールとタブに印が出て、窓の枠が光ります。", theme))
                .child(body(route, theme))
                .child(body(send, theme))
                .child(div().pt(px(12.)).child(body("これらを使うには、Claude Code に fude の hook を登録します。登録しなくても次へ進めます。", theme)))
                .child(div().pt(px(6.)).child(self.hooks.clone()));
        }
        el
    }

    fn render_ready(&self, theme: &Theme) -> Div {
        div()
            .child(title("準備ができました", theme))
            .child(lead_text("あとから見直すときは、コマンドパレットの「チュートリアルをもう一度」から開けます。", theme))
    }
}

/// 2 ページ目などに出すキーの表示。今の割り当てから作る（§5.2）。
struct Hints {
    tree: KeyHints,
    go_to_file: Option<String>,
    palette: Option<String>,
    settings: Option<String>,
    route_map: Option<String>,
    send_to_claude: Option<String>,
}

impl Hints {
    fn from_settings(settings: &fude_core::settings::Settings) -> Hints {
        let platform = Platform::current();
        let (list, _) = crate::workspace_view::valid_bindings(settings);
        let phrase = |c: Command| keys_phrase(&key_hints(&list, c, platform));
        // Claude に渡すはファイルの中だけで効くので、文脈を問わず最初のキーを使う。
        let send = list.iter().find(|b| b.command == Command::SendToClaude).map(|b| key_label(&b.key, platform));
        Hints {
            tree: key_hints(&list, Command::ToggleTree, platform),
            go_to_file: phrase(Command::GoToFile),
            palette: phrase(Command::OpenPalette),
            settings: phrase(Command::OpenSettings),
            route_map: phrase(Command::ToggleRouteMap),
            send_to_claude: send,
        }
    }
}

fn title(text: &'static str, theme: &Theme) -> Div {
    div().pt(px(8.)).pb(px(8.)).text_size(px(20.)).text_color(theme.fg_default).child(text)
}

fn lead_text(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div().pb(px(12.)).text_color(theme.fg_dim).child(text.into())
}

fn body(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div().py(px(2.)).text_color(theme.fg_default).child(text.into())
}

/// 2 ページ目の 1 項目（見出しと中身）。
fn item(name: &'static str, content: impl IntoElement, theme: &Theme) -> Div {
    div().pt(px(14.)).child(div().pb(px(4.)).text_color(theme.accent).child(name)).child(content)
}

/// 進むボタン（アクセント色の枠）。
fn primary_button(id: &'static str, label: &'static str, theme: &Theme) -> Stateful<Div> {
    small_button(id, label, theme).border_color(theme.accent).text_color(theme.accent)
}

impl Render for TutorialView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let hints = Hints::from_settings(&cx.global::<ConfigState>().settings);
        if self.page == Page::Basics && !self.reduced_motion {
            window.request_animation_frame();
        }
        let page = match self.page {
            Page::Look => self.render_look(&hints, &theme),
            Page::Basics => self.render_basics(&hints, &theme),
            Page::Claude => self.render_claude(&hints, &theme, cx),
            Page::Ready => self.render_ready(&theme),
        };
        let skip = small_button("tutorial-skip", "スキップ", &theme)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(TutorialEvent::Finished)));
        let dots = div().flex().flex_row().gap(px(6.)).children(PAGES.iter().map(|p| {
            div().size(px(7.)).rounded(px(3.5)).bg(if *p == self.page { theme.accent } else { theme.border_subtle })
        }));
        let last = self.page == Page::Ready;
        let next = primary_button("tutorial-next", if last { "はじめる" } else { "次へ" }, &theme)
            .on_click(cx.listener(|this, _, _, cx| this.go(Nav::Next, cx)));
        let back = (self.page != Page::Look).then(|| {
            small_button("tutorial-back", "戻る", &theme).on_click(cx.listener(|this, _, _, cx| this.go(Nav::Back, cx)))
        });
        let column = div()
            .w_full()
            .max_w(px(640.))
            .h_full()
            .px(px(24.))
            .flex()
            .flex_col()
            .child(div().flex_shrink_0().py(px(10.)).flex().flex_row().justify_end().child(skip))
            .child(
                div()
                    .id("tutorial-page")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pb(px(16.))
                    .child(page),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .py(px(12.))
                    .border_t_1()
                    .border_color(theme.border_subtle)
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(dots)
                    .child(div().flex_1())
                    .child(div().flex().flex_row().gap(px(8.)).children(back).child(next)),
            );
        div()
            .id("tutorial")
            .track_focus(&self.focus)
            .key_context("Tutorial")
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                // 入力欄やボタンにフォーカスがあるときは送らない（← → は入力欄のカーソルの移動に使う）。
                if !this.focus.is_focused(window) || ev.keystroke.modifiers.modified() {
                    return;
                }
                match ev.keystroke.key.as_str() {
                    "left" => this.go(Nav::Back, cx),
                    "right" | "enter" => this.go(Nav::Next, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .text_size(px(13.))
            .text_color(theme.fg_default)
            .child(column)
    }
}

#[cfg(test)]
mod tests {
    use super::{LeaveFocus, Nav, Page, closes_tutorial, decide_startup, key_command_closes, leave_focus, step, tree_text, write_mark};
    use fude_core::keymap::Command;
    use fude_core::palette::PaletteCommand;

    #[test]
    fn leave_tutorial_on_command() {
        assert!(!closes_tutorial(PaletteCommand::ShowTutorial));
        assert!(!closes_tutorial(PaletteCommand::OpenSettings));
        for c in [PaletteCommand::ToggleTree, PaletteCommand::GoToFile, PaletteCommand::OpenFolder, PaletteCommand::NewTerminal] {
            assert!(closes_tutorial(c), "{c:?}");
        }
    }

    #[test]
    fn leave_tutorial_on_key() {
        assert!(!key_command_closes(Command::OpenPalette, true), "パレットを開くだけでは閉じない");
        assert!(!key_command_closes(Command::OpenSettings, true), "チュートリアルの上の Ctrl+, は何もしない");
        for c in [Command::ToggleTree, Command::GoToFile, Command::NewTab, Command::ActivateProject(2), Command::FindInFiles] {
            assert!(key_command_closes(c, true), "{c:?}");
        }
    }

    #[test]
    fn key_without_project_keeps_tutorial() {
        // プロジェクトが無いときは、どのキーの命令も画面を変えないので閉じない（2 ページ目で Ctrl+B を試しても消えない）。
        for c in Command::FIXED.into_iter().chain((1..=9).map(Command::ActivateProject)) {
            assert!(!key_command_closes(c, false), "{c:?}");
        }
    }

    #[test]
    fn leave_focus_only_when_tutorial_owned_it() {
        // 命令がフォーカスを移していれば（パレットの欄、ツリーなど）、そのままにする。
        assert_eq!(leave_focus(false, true), LeaveFocus::Keep);
        assert_eq!(leave_focus(false, false), LeaveFocus::Keep);
        // チュートリアルがフォーカスを持っていたなら、すぐに選択中のターミナルへ（次の描画まで待たない）。
        assert_eq!(leave_focus(true, true), LeaveFocus::Terminal);
        assert_eq!(leave_focus(true, false), LeaveFocus::Workspace);
    }
    use fude_core::keymap::KeyHints;
    use fude_core::tutorial::{StartupTutorial, is_done};

    #[test]
    fn step_moves_between_pages() {
        assert_eq!(step(Page::Look, Nav::Next), Some(Page::Basics));
        assert_eq!(step(Page::Basics, Nav::Next), Some(Page::Claude));
        assert_eq!(step(Page::Basics, Nav::Back), Some(Page::Look));
        assert_eq!(step(Page::Claude, Nav::Next), Some(Page::Ready));
        assert_eq!(step(Page::Ready, Nav::Back), Some(Page::Claude));
    }

    #[test]
    fn step_ends() {
        assert_eq!(step(Page::Look, Nav::Back), None);
        assert_eq!(step(Page::Ready, Nav::Next), None);
    }

    #[test]
    fn tree_line_omits_missing_keys() {
        let both = KeyHints { anywhere: Some("Ctrl+Shift+B".into()), outside_terminal: Some("Ctrl+B".into()) };
        assert_eq!(tree_text(&both), "上のプロジェクトのピルをクリックするか、Ctrl+B を押します（ターミナルの中では Ctrl+Shift+B）。");
        let anywhere = KeyHints { anywhere: Some("Ctrl+Alt+T".into()), outside_terminal: None };
        assert_eq!(tree_text(&anywhere), "上のプロジェクトのピルをクリックするか、Ctrl+Alt+T を押します。");
        assert_eq!(tree_text(&KeyHints::default()), "上のプロジェクトのピルをクリックします。");
    }

    #[test]
    fn startup_decision_for_existing_user() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("fude").join("tutorial.json");
        assert_eq!(decide_startup(true, Some(&mark), false), StartupTutorial::MarkDone);
        assert!(is_done(std::fs::read_to_string(&mark).ok().as_deref()));
        // 印が付いたので、次は出さない。
        assert_eq!(decide_startup(true, Some(&mark), true), StartupTutorial::Hide);
    }

    #[test]
    fn startup_decision_for_new_user() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("tutorial.json");
        assert_eq!(decide_startup(true, Some(&mark), true), StartupTutorial::Show);
        assert!(!mark.exists(), "出すときは印を付けない");
        assert_eq!(decide_startup(false, Some(&mark), true), StartupTutorial::Hide);
    }

    #[test]
    fn write_mark_reports_error_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file.txt");
        std::fs::write(&blocker, "").unwrap();
        assert!(write_mark(&blocker.join("tutorial.json")).is_err());
    }
}
