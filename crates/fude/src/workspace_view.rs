//! ルートのビュー。全体の配置と、アクションの処理を受け持つ。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fude_core::keymap::{Binding, Command, Platform, bindings};
use fude_core::recent::RecentList;
use fude_core::session::{self, SessionData, WindowGeom};
use fude_core::settings::{ConfigIssue, LastProjectClosed, Settings};
use fude_core::claude::{AgentState, Agents, HookCommand, HookSignal, hook_config, parse_terminal_id};
use fude_core::workspace::{OpenMode, PaneId, Project, ProjectId, ProjectRoot, ShellKind, Tab, Workspace};
use fude_core::motion::Fade;
use fude_ipc::Message;
use gpui_kit::*;

use crate::hover::{HoverFades, HoverKey};
use crate::main_area::render_main_area;
use crate::popup::Popup;
use crate::terminal_view::{TerminalEvent, TerminalView};
use crate::theme::motion::{SCREEN_FADE, ease_out};
use crate::theme::{Theme, ui_font};
use crate::wsl_host::{WslHosts, project_fs};

gpui_kit::actions!(
    fude,
    [
        NewTab,
        SplitTab,
        NextTab,
        PrevTab,
        ActivateProject1,
        ActivateProject2,
        ActivateProject3,
        ActivateProject4,
        ActivateProject5,
        ActivateProject6,
        ActivateProject7,
        ActivateProject8,
        ActivateProject9,
        CopySelection,
        PasteText,
        CloseItem,
        OpenPalette,
        GoToFile,
        FindInFiles,
        FindInTerminal,
        ToggleRouteMap,
    ]
);

/// Command の GPUI のアクション。
fn command_action(command: Command) -> Box<dyn Action> {
    use crate::file_view::{SaveFile, SendToClaude, ToggleMarkdownMode};
    use crate::tree_view::ToggleTree;
    match command {
        Command::Close => Box::new(CloseItem),
        Command::ToggleTree => Box::new(ToggleTree),
        Command::Save => Box::new(SaveFile),
        Command::SendToClaude => Box::new(SendToClaude),
        Command::ToggleMarkdownMode => Box::new(ToggleMarkdownMode),
        Command::OpenPalette => Box::new(OpenPalette),
        Command::GoToFile => Box::new(GoToFile),
        Command::FindInFiles => Box::new(FindInFiles),
        Command::FindInTerminal => Box::new(FindInTerminal),
        Command::ToggleRouteMap => Box::new(ToggleRouteMap),
        Command::NewTab => Box::new(NewTab),
        Command::SplitTab => Box::new(SplitTab),
        Command::NextTab => Box::new(NextTab),
        Command::PrevTab => Box::new(PrevTab),
        Command::Copy => Box::new(CopySelection),
        Command::Paste => Box::new(PasteText),
        Command::ActivateProject(n) => match n {
            1 => Box::new(ActivateProject1),
            2 => Box::new(ActivateProject2),
            3 => Box::new(ActivateProject3),
            4 => Box::new(ActivateProject4),
            5 => Box::new(ActivateProject5),
            6 => Box::new(ActivateProject6),
            7 => Box::new(ActivateProject7),
            8 => Box::new(ActivateProject8),
            _ => Box::new(ActivateProject9),
        },
    }
}

/// キーと文脈とアクションから KeyBinding を作る。KeyBinding::new は &'static str しか受けないため load を使う。
fn load_binding(key: &str, action: Box<dyn Action>, context: Option<&'static str>, cx: &App) -> Option<KeyBinding> {
    let predicate = context.map(|c| std::rc::Rc::new(KeyBindingContextPredicate::parse(c).expect("文脈の表記")));
    KeyBinding::load(key, action, predicate, false, None, cx).ok()
}

/// GPUI が読めるキーの表記か（空白で区切った連続のキーも含む）。KeyBinding::new は読めない表記で落ちるため、先に確かめる。
pub(crate) fn valid_key(key: &str, os: OperatingSystem) -> bool {
    let mut parts = key.split_whitespace().peekable();
    parts.peek().is_some() && parts.all(|k| Keystroke::parse_with_os(k, os).is_ok())
}

fn current_os() -> OperatingSystem {
    if cfg!(target_os = "macos") { OperatingSystem::Mac } else { OperatingSystem::Windows }
}

/// 設定の上書きを当てた割り当て表のうち、読めるキーだけ。読めないキーは issue にする。
pub(crate) fn valid_bindings(settings: &Settings) -> (Vec<Binding>, Vec<ConfigIssue>) {
    let (list, mut issues) = bindings(Platform::current(), &settings.keybindings);
    let (ok, bad): (Vec<Binding>, Vec<Binding>) = list.into_iter().partition(|b| valid_key(&b.key, current_os()));
    issues.extend(bad.into_iter().map(|b| ConfigIssue { line: None, message: format!("keybindings の \"{}\" は読めないキーの表記です（{}）", b.key, b.command.name()) }));
    (ok, issues)
}

/// fude_core の割り当て表（設定の上書きを当てたもの）を GPUI のキーバインドに変換して登録する。
pub fn bind_keys(cx: &mut App, settings: &Settings) -> Vec<ConfigIssue> {
    let (list, issues) = valid_bindings(settings);
    let keys: Vec<KeyBinding> = list.iter().filter_map(|b| load_binding(&b.key, command_action(b.command), b.context, cx)).collect();
    cx.bind_keys(keys);
    issues
}

/// 設定が変わったときに割り当てを登録し直す。gpui-kit の部品のキーを消さないよう clear_key_bindings は使わず、
/// 使わなくなったキーは NoAction で外し、新しい表を後から足す（後から足したものが優先される）。
pub fn rebind_keys(cx: &mut App, old: &Settings, new: &Settings) -> Vec<ConfigIssue> {
    let (new_list, issues) = valid_bindings(new);
    if old.keybindings == new.keybindings {
        return issues;
    }
    let (old_list, _) = valid_bindings(old);
    let removed: Vec<KeyBinding> = old_list
        .iter()
        .filter(|o| !new_list.iter().any(|n| n.key == o.key && n.context == o.context))
        .filter_map(|o| load_binding(&o.key, Box::new(NoAction), o.context, cx))
        .collect();
    cx.bind_keys(removed);
    let added: Vec<KeyBinding> = new_list.iter().filter_map(|b| load_binding(&b.key, command_action(b.command), b.context, cx)).collect();
    cx.bind_keys(added);
    issues
}

/// ブランチ名の読み方。
#[derive(Debug, PartialEq)]
pub(crate) enum BranchRead {
    /// その場で読む（ローカル）。
    Now,
    /// 裏で fude serve から読む（WSL。止まっている WSL の起動や fude の配置に数秒かかり、UI を止めてしまうため）。
    Background,
}

pub(crate) fn branch_read(root: &ProjectRoot) -> BranchRead {
    match root {
        ProjectRoot::Local(_) => BranchRead::Now,
        ProjectRoot::Wsl { .. } => BranchRead::Background,
    }
}

/// プロジェクトを閉じたあとに行うこと。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AfterClose {
    /// まだプロジェクトがある。
    Stay,
    /// ホーム画面を出す（プロジェクトが 0 個なら自然に出る）。
    ShowHome,
    /// アプリを終える。
    Quit,
}

/// remaining は閉じたあとのプロジェクトの数。
pub(crate) fn last_project_closed_action(settings: &Settings, remaining: usize) -> AfterClose {
    match (remaining, settings.close.on_last_project) {
        (1.., _) => AfterClose::Stay,
        (0, LastProjectClosed::Home) => AfterClose::ShowHome,
        (0, LastProjectClosed::Quit) => AfterClose::Quit,
    }
}

/// 起動のときに開くもの（main の Launch から）。
pub enum InitialOpen {
    Nothing,
    Project { root: ProjectRoot, shell: Option<ShellKind> },
    File { file: fude_core::location::FileLocation, root_hint: Option<ProjectRoot> },
}

/// 起動のときに読んだもの。
pub struct Startup {
    pub session_path: Option<PathBuf>,
    /// 前回の続き（ホーム画面の「前回の続き」で戻す）。
    pub last_session: Option<SessionData>,
    pub recent_path: Option<PathBuf>,
    pub recent: RecentList,
    pub open: InitialOpen,
}

/// 今の UNIX 秒。
fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// ペインの FUDE_TERMINAL_ID。
pub(crate) fn terminal_id(pane: PaneId) -> String {
    format!("p{}", pane.raw())
}

/// 「Claude Code を起動」で、新しいターミナルを作った直後に書く入力。
pub const CLAUDE_LAUNCH_INPUT: &[u8] = b"claude\r";

/// 作業中のプロジェクトに合う hook のコマンド。WSL は Shell(WSL 側の fude の置き場所)、それ以外は
/// Exec(この fude の / 区切りの絶対パス)。
pub(crate) fn hook_command_for(root: &ProjectRoot, exe: &std::path::Path, wsl_fude: &str) -> HookCommand {
    match root {
        ProjectRoot::Wsl { .. } => HookCommand::Shell(wsl_fude.to_string()),
        ProjectRoot::Local(_) => HookCommand::Exec(exe.to_string_lossy().replace('\\', "/")),
    }
}

/// 状態が変わってから保存するまでの待ち時間。続けて変わったら、最後の変化から数え直す。
const SAVE_DELAY: Duration = Duration::from_secs(1);

/// GPUI の WindowBounds から保存する形へ。Windowed は maximized=false、Maximized と Fullscreen は true（大きさは元に戻したとき）。
pub(crate) fn geom_from_bounds(bounds: WindowBounds) -> WindowGeom {
    let (b, maximized) = match bounds {
        WindowBounds::Windowed(b) => (b, false),
        WindowBounds::Maximized(b) | WindowBounds::Fullscreen(b) => (b, true),
    };
    WindowGeom {
        x: f32::from(b.origin.x),
        y: f32::from(b.origin.y),
        width: f32::from(b.size.width),
        height: f32::from(b.size.height),
        maximized,
    }
}

pub struct WorkspaceView {
    pub(crate) workspace: Workspace,
    /// ペインごとの Claude Code の状態（返事待ち、タスク）。hook の信号で変わる。
    pub(crate) agents: Agents,
    /// 返事待ちの枠の濃さ（0〜1）。
    pub(crate) wait_frame: Fade,
    /// 最後に OS に出したバッジの数。
    last_badge: usize,
    /// プロジェクトごとのプランの読み取り（路線図）。
    pub(crate) plans: HashMap<ProjectId, crate::route_map::PlanCache>,
    /// プランを確かめている最中のプロジェクト。
    pub(crate) plan_checking: HashSet<ProjectId>,
    /// 路線図の折りたたみ（全プロジェクトで 1 つ。保存しない）。
    pub(crate) route_collapsed: bool,
    /// 路線図の幅（px）と、実行中の輪の位置（駅の番号）。
    pub(crate) route_w: Fade,
    pub(crate) ring: Fade,
    /// 前の描画で輪を出していたか（初めて出るときは動かさない）。
    pub(crate) ring_shown: bool,
    /// 前の描画の作業中のプロジェクト（変わったらすぐプランを確かめる）。
    pub(crate) route_project: Option<ProjectId>,
    /// 最後に出した路線図。消えるときの動き（幅が 0 に戻るまで）の間、これを描く。
    pub(crate) route_last: Option<(ProjectId, fude_core::plan::Route)>,
    pub(crate) plan_poll: Option<Task<()>>,
    /// ピル用のブランチ名。開いたときと、ウィンドウが前面に来たときに読み直す。
    pub(crate) branches: HashMap<ProjectId, Option<String>>,
    pub(crate) hover: HoverFades,
    /// macOS でタイトル行をドラッグしてウィンドウを動かすための状態。
    pub(crate) should_move: bool,
    /// ペインごとのターミナル。表示したときに作り、ペインが無くなったら捨てる（子プロセスも終わる）。
    terminals: HashMap<PaneId, (Entity<TerminalView>, Subscription)>,
    /// 最後にフォーカスを当てたペイン。選択が変わったら、新しいペインへフォーカスを移す。
    focused_pane: Option<PaneId>,
    /// ペインのターミナルを作った直後に書く入力（「Claude Code を起動」）。
    pending_input: HashMap<PaneId, Vec<u8>>,
    /// ＋の右クリックメニュー。
    pub(crate) shell_menu: Option<Popup>,
    /// レールの右クリックメニューと、その対象のプロジェクト。
    pub(crate) project_menu: Option<(ProjectId, Popup)>,
    /// インストール済みの WSL のディストリビューション。起動時に 1 回だけ読む。
    pub(crate) distros: Vec<String>,
    /// WSL のプロジェクトのファイルへのアクセス（ディストリビューションごとの fude serve）。
    pub(crate) wsl_hosts: Arc<WslHosts>,
    pub(crate) focus: FocusHandle,
    /// セッションの保存先。環境変数から決まらなければ保存しない。
    session_path: Option<PathBuf>,
    /// 最後に書いた内容と、書く予定の内容。同じ内容を何度も書かないために持つ。
    saved: Option<SessionData>,
    pending: Option<SessionData>,
    /// 書く予定の処理。差し替えると前の予定は取り消される。
    save_task: Option<Task<()>>,
    /// 最後に見たウィンドウの位置と大きさ（アプリの終了時はウィンドウに触れないため）。
    last_geom: Option<WindowGeom>,
    /// 設定の写し（ConfigState と同じ値。変化を見比べるために持つ）。
    pub(crate) settings: Settings,
    /// 閉じた書き損じの知らせ（同じ内容なら出し直さない）。
    notice_dismissed: Option<Vec<ConfigIssue>>,
    _config: Subscription,
    /// 最近開いたプロジェクトと、その保存先。
    pub(crate) recent: RecentList,
    recent_path: Option<PathBuf>,
    /// 「前回の続き」で戻す状態。最後にプロジェクトが 1 個以上あった時点のもの。
    pub(crate) last_session: Option<SessionData>,
    /// ホーム画面で薄く出す（フォルダが見つからない）プロジェクト。
    pub(crate) missing_roots: HashSet<ProjectRoot>,
    /// 右パネルに開いたあとで、カーソルを置く位置（プロジェクト、1 始まりの行と列）。
    /// ファイルツリー（左パネル）。
    pub(crate) tree: crate::tree_view::TreeState,
    /// プロジェクトごとの ProjectFs、監視、Git の状態。
    pub(crate) live: HashMap<ProjectId, crate::project_state::ProjectLive>,
    /// 変更したファイルの一覧（「● N」を押すと出る）。
    pub(crate) changes_menu: Option<(ProjectId, crate::popup::Popup)>,
    /// プロジェクトごとの右パネル。
    pub(crate) editors: HashMap<ProjectId, crate::editor_column::EditorSlot>,
    /// エディタ列のタブ列のスクロールと、最後に寄せたタブ。
    pub(crate) editor_tab_scroll: (ScrollHandle, Option<(ProjectId, usize)>),
    /// 右パネルの出入りとエディタの移動の動き。
    pub(crate) editor_anim: crate::editor_column::EditorAnim,
    /// 左パネル（ツリー）の出入り。
    pub(crate) tree_presence: crate::anim::Presence,
    /// ピン列（プロジェクトごとのビューと、出入りの動き）。
    pub(crate) pins: crate::pin_view::PinSlot,
    /// ターミナルのタブを縦に並べるか（全プロジェクトで 1 つ。session.json に保存し、起動時に常に読む）。
    pub(crate) vertical_tabs: bool,
    /// 縦のタブの列の幅の動き。
    pub(crate) vtabs_w: Fade,
    /// タブと分割の動き。
    pub(crate) anims: crate::layout_anim::LayoutAnims,
    /// OS の「動きを減らす」設定。起動時とウィンドウが前面に来たときに読む。
    pub(crate) reduced_motion: bool,
    /// 確かめ終えて、ウィンドウを閉じてよい（× の 2 回目）。
    pub(crate) allow_close: bool,
    /// 確認ダイアログ。
    pub(crate) dialog: Option<crate::dialog::OpenDialog>,
    /// プロジェクトごとの全体検索の結果のタブ。
    pub(crate) searches: HashMap<ProjectId, (Entity<crate::search_view::SearchView>, Subscription)>,
    /// コマンドパレット。
    pub(crate) palette: Option<crate::palette_view::PaletteState>,
    /// ホーム画面の「最近」の右クリックメニュー。
    pub(crate) home_menu: Option<(ProjectRoot, crate::popup::Popup)>,
    /// ホーム画面とプロジェクトの画面の切り替えのフェード（新しい画面の不透明度）。
    screen_fade: Fade,
    /// 前の描画でホーム画面を出していたか。
    showing_home: Option<bool>,
    _activation: Subscription,
    _bounds: Subscription,
    _quit: Subscription,
}

impl WorkspaceView {
    /// 起動時にセッションは戻さない（設計書 §5.10 の B 案）。startup.open があれば開く。
    pub fn new(startup: Startup, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let Startup { session_path, last_session, recent_path, recent, open } = startup;
        let vertical_tabs = last_session.as_ref().is_some_and(|s| s.vertical_tabs);
        let workspace = Workspace::default();
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.reduced_motion = crate::reduced_motion::reduced_motion();
                this.refresh_active_branch(cx);
                if let Some(id) = this.workspace.active_project().map(|p| p.id) {
                    this.refresh_status(id, std::time::Duration::ZERO, cx);
                }
                cx.notify();
            }
        });
        let bounds = cx.observe_window_bounds(window, |this, window, cx| this.schedule_save(window, cx));
        let quit = cx.on_app_quit(|this, _| {
            this.save_now();
            async {}
        });
        let view = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let geom = geom_from_bounds(window.window_bounds());
            // 未保存の変更と動いているターミナルを確かめてから閉じる（設計書 §5.12）。確かめ終えたら allow_close。
            view.update(cx, |this, cx| {
                this.last_geom = Some(geom);
                if this.allow_close {
                    this.save_now();
                    return true;
                }
                this.request_quit(window, cx);
                false
            })
            .unwrap_or(true)
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            workspace,
            agents: Agents::default(),
            wait_frame: Fade::settled(0.0),
            last_badge: 0,
            plans: HashMap::new(),
            plan_checking: HashSet::new(),
            route_collapsed: false,
            route_w: Fade::settled(0.0),
            ring: Fade::settled(0.0),
            ring_shown: false,
            route_project: None,
            route_last: None,
            plan_poll: None,
            branches: HashMap::new(),
            hover: HoverFades::default(),
            should_move: false,
            terminals: HashMap::new(),
            focused_pane: None,
            pending_input: HashMap::new(),
            shell_menu: None,
            project_menu: None,
            distros: Vec::new(),
            wsl_hosts: Arc::new(WslHosts::new()),
            focus,
            session_path,
            settings: cx.global::<crate::config_runtime::ConfigState>().settings.clone(),
            notice_dismissed: None,
            _config: cx.observe_global_in::<crate::config_runtime::ConfigState>(window, |this, window, cx| this.on_config_changed(window, cx)),
            recent,
            recent_path,
            last_session,
            missing_roots: HashSet::new(),
            home_menu: None,
            palette: None,
            searches: HashMap::new(),
            dialog: None,
            allow_close: false,
            reduced_motion: crate::reduced_motion::reduced_motion(),
            anims: Default::default(),
            editor_anim: Default::default(),
            tree_presence: crate::anim::Presence::hidden(),
            pins: Default::default(),
            vertical_tabs,
            vtabs_w: Fade::settled(0.0),
            editors: HashMap::new(),
            editor_tab_scroll: (ScrollHandle::new(), None),
            live: HashMap::new(),
            tree: crate::tree_view::TreeState::new(cx),
            changes_menu: None,
            screen_fade: Fade::settled(1.0),
            showing_home: None,
            saved: None,
            pending: None,
            save_task: None,
            last_geom: None,
            _activation: activation,
            _bounds: bounds,
            _quit: quit,
        };
        this.workspace.set_local_shell(&this.settings.terminal);
        match open {
            InitialOpen::Project { root, shell } => this.open_project(root, shell, cx),
            InitialOpen::File { file, root_hint } => this.open_file(file, root_hint, None, window, cx),
            InitialOpen::Nothing => {}
        }
        this.refresh_active_branch(cx);
        this.load_distros(cx);
        this.start_plan_poll(cx);
        this
    }

    /// 最近開いたものに記録して、裏で保存する。
    pub(crate) fn touch_recent(&mut self, root: &ProjectRoot, cx: &mut Context<Self>) {
        self.recent.touch(root.clone(), now_secs());
        self.save_recent(cx);
    }

    pub(crate) fn save_recent(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.recent_path.clone() else { return };
        let text = self.recent.to_json();
        cx.background_spawn(async move {
            if let Err(e) = session::write_atomic(&path, &text) {
                eprintln!("最近開いたものを保存できない: {e}");
            }
        })
        .detach();
    }

    /// 「前回の続き」。プロジェクトが 0 個のときだけ、保存した状態を戻す（ターミナルも保存どおりに起動する）。
    pub(crate) fn resume_last_session(&mut self, cx: &mut Context<Self>) {
        if !self.workspace.projects().is_empty() {
            return;
        }
        let Some(data) = self.last_session.clone() else { return };
        self.workspace = Workspace::restore(&data, &|p| p.is_dir());
        // 戻したプロジェクトも最近開いたものに記録する。選択中のものを最後にして、一覧の先頭にする。
        let active = self.workspace.active_project().map(|p| p.id);
        let mut roots: Vec<(bool, ProjectRoot)> =
            self.workspace.projects().iter().map(|p| (Some(p.id) == active, p.root.clone())).collect();
        roots.sort_by_key(|(is_active, _)| *is_active);
        let now = now_secs();
        for (_, root) in roots {
            self.recent.touch(root, now);
        }
        self.save_recent(cx);
        // 戻したタブのビューは、選ばれたときに作る（描画の ensure_active_view）。
        self.refocus_pane();
        self.refresh_active_branch(cx);
        cx.notify();
    }

    /// プロジェクトを閉じたあとに呼ぶ。0 個になり、設定がアプリの終了なら、保存して終える。
    pub(crate) fn after_project_closed(&mut self, cx: &mut Context<Self>) {
        if last_project_closed_action(&self.settings, self.workspace.projects().len()) == AfterClose::Quit {
            self.save_now();
            cx.quit();
        }
    }

    fn load_distros(&self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async { fude_term::list_distros() });
        cx.spawn(async move |this, cx| {
            let distros = match task.await {
                Ok(d) => d,
                Err(err) => {
                    eprintln!("WSL のディストリビューションを読めない: {err}");
                    return;
                }
            };
            this.update(cx, |this, cx| {
                this.distros = distros;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// エディタ列のビューにフォーカスを当てる（ペインへの戻しを止める）。
    pub(crate) fn focus_editor(&mut self, focus: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        self.focused_pane = None;
        window.focus(focus, cx);
    }

    /// メニューを閉じたあと、フォーカスを戻す。ペインがあればそこへ（次の描画で）、無ければ右パネル、
    /// それも無ければワークスペースへ。
    pub(crate) fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let has_pane = self.workspace.active_project().and_then(|p| p.active_tab()).is_some();
        if has_pane {
            self.refocus_pane();
        } else if let Some(focus) = self.active_editor_focus(cx) {
            self.focused_pane = None;
            window.focus(&focus, cx);
        } else {
            self.focused_pane = None;
            window.focus(&self.focus, cx);
        }
    }

    /// Claude Code の hook の信号をペインに当てる。新しく返事待ちになったら true。知らないペインは無視する。
    pub(crate) fn on_hook(&mut self, terminal: &str, signal: HookSignal, window: &mut Window, _cx: &mut Context<Self>) -> bool {
        let Some((_, pane)) = parse_terminal_id(terminal).and_then(|raw| self.workspace.pane_by_raw(raw)) else { return false };
        let became = self.agents.apply(pane.raw(), signal);
        if crate::attention::should_request_attention(became, window.is_window_active()) {
            crate::attention::request_attention(window);
        }
        became
    }

    /// 返事待ちの枠を動かし、タスクバー／Dock のバッジを数に合わせる。
    fn observe_waiting(&mut self, window: &mut Window, now: Instant) {
        use crate::theme::motion::{REDUCED, WAIT_IN, WAIT_OUT, ease_in_out};
        let count = self.agents.waiting_count();
        let (target, duration) = if count > 0 { (1.0, WAIT_IN) } else { (0.0, WAIT_OUT) };
        let duration = if self.reduced_motion { REDUCED } else { duration };
        self.wait_frame.animate_to(target, duration, ease_in_out, now);
        if self.wait_frame.is_animating(now) {
            window.request_animation_frame();
        }
        if count != self.last_badge {
            self.last_badge = count;
            crate::attention::set_badge(window, count);
        }
    }

    /// どこかに返事待ちがある間、ウィンドウの縁に出す warn の枠（にじませない）。
    fn render_wait_frame(&self, theme: &Theme, now: Instant) -> Option<Div> {
        let v = self.wait_frame.value(now);
        (v > 0.0).then(|| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .rounded(crate::theme::size::WINDOW_CORNER)
                .border(crate::theme::size::WAIT_FRAME_W)
                .border_color(Rgba { a: theme.warn.a * v, ..theme.warn })
        })
    }

    pub(crate) fn pane_waiting(&self, pane: PaneId) -> bool {
        self.agents.state(pane.raw()) == AgentState::Waiting
    }

    pub(crate) fn tab_waiting(&self, tab: &Tab) -> bool {
        tab.panes.iter().any(|p| self.pane_waiting(p.id))
    }

    pub(crate) fn project_waiting(&self, project: &Project) -> bool {
        project.tabs.iter().any(|t| self.tab_waiting(t))
    }

    /// そのプロジェクトの選択中のタブの、フォーカス中のペインに text を書き、そこへフォーカスを移す。ペインが無ければ何もしない。
    /// Enter は送らない。入力したので、そのペインの返事待ちを消す。
    pub(crate) fn send_to_claude(&mut self, project: ProjectId, text: String, cx: &mut Context<Self>) {
        let Some(p) = self.workspace.projects().iter().find(|p| p.id == project) else { return };
        let Some(pane) = p.active_tab().map(|t| t.panes[t.focused].id) else { return };
        let Some((view, _)) = self.terminals.get(&pane) else { return };
        view.read(cx).write(text.into_bytes());
        self.agents.typed(pane.raw());
        self.workspace.focus_pane(pane);
        // 次の描画の sync_terminals がそのペインへフォーカスを当てる。
        self.refocus_pane();
        cx.notify();
    }

    /// 設定ファイルを読み直した（ConfigState が変わった）。ウィンドウとターミナルとプロジェクトに効くものを反映する。
    /// テーマ、フォント、キーは config_runtime::apply_global が反映済み。
    fn on_config_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new = cx.global::<crate::config_runtime::ConfigState>().settings.clone();
        if new == self.settings {
            cx.notify();
            return;
        }
        let old = std::mem::replace(&mut self.settings, new.clone());
        if old.window != new.window {
            window.set_background_appearance(crate::config_runtime::background_appearance(&new.window));
        }
        self.workspace.set_local_shell(&new.terminal);
        for (view, _) in self.terminals.values() {
            view.update(cx, |v, cx| v.apply_settings(&new, window, cx));
        }
        if old.tree != new.tree {
            // 隠すフォルダが変わったら、ファイルへのアクセスと監視を作り直し、ツリーを読み直す。
            self.live.clear();
            self.tree.models.clear();
            self.sync_live(cx);
        }
        cx.notify();
    }

    /// 隠すフォルダの設定。
    pub(crate) fn heavy_dirs(&self) -> fude_fs::ignored::HeavyDirs {
        fude_fs::ignored::HeavyDirs::from_settings(self.settings.tree.heavy_dirs.clone(), self.settings.tree.show_heavy_dirs)
    }

    /// 設定ファイルの書き損じの知らせ（最下部の 1 行）。最初の 1 件と、ほかの件数、「開く」、閉じる。
    fn render_config_notice(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let state = cx.global::<crate::config_runtime::ConfigState>();
        let first = state.issues.first()?;
        if self.notice_dismissed.as_ref() == Some(&state.issues) {
            return None;
        }
        let rest = state.issues.len() - 1;
        let text = if rest > 0 {
            format!("{}（ほかに {rest} 件）", crate::config_runtime::issue_text(first))
        } else {
            crate::config_runtime::issue_text(first)
        };
        let path = state.path.clone();
        let issues = state.issues.clone();
        let button = |label: &'static str| {
            div().px(px(8.)).py(px(2.)).rounded(crate::theme::size::RADIUS_SM).cursor_pointer().hover(|s| s.bg(theme.bg_hover)).child(label)
        };
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(4.))
                .bg(theme.bg_panel)
                .border_t_1()
                .border_color(theme.border_subtle)
                .text_size(px(12.))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(theme.warn).child(text))
                .children(path.map(|path| {
                    button("開く").id("config-notice-open").on_click(cx.listener(move |this, _, window, cx| {
                        this.open_file(fude_core::location::FileLocation::Local(path.clone()), None, None, window, cx);
                    }))
                }))
                .child(button("閉じる").id("config-notice-close").on_click(cx.listener(move |this, _, _, cx| {
                    this.notice_dismissed = Some(issues.clone());
                    cx.notify();
                }))),
        )
    }

    /// 既定のシェルで新しいタブを開き、`claude` と Enter を入力する。
    pub(crate) fn launch_claude(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.workspace.active_project().map(|p| self.workspace.new_terminal_shell(p)) else { return };
        if self.workspace.new_tab(kind).is_none() {
            return;
        }
        if let Some(pane) = self.workspace.active_project().and_then(|p| p.active_tab()).map(|t| t.panes[t.focused].id) {
            self.pending_input.insert(pane, CLAUDE_LAUNCH_INPUT.to_vec());
        }
        cx.notify();
    }

    /// 作業中のプロジェクトに合う hook の設定を、settings.json に貼る形でクリップボードに書き、貼る場所を知らせる。
    pub(crate) fn copy_hook_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.active_project().map(|p| p.root.clone()) else { return };
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                self.show_error(format!("fude の場所が分からないため、hook の設定を作れません: {e}"), window, cx);
                return;
            }
        };
        #[cfg(windows)]
        let wsl_fude = self.wsl_hosts.bin().to_string();
        #[cfg(not(windows))]
        let wsl_fude = String::new();
        let config = hook_config(&hook_command_for(&root, &exe, &wsl_fude));
        let place = match root {
            ProjectRoot::Wsl { .. } => "WSL の中の ~/.claude/settings.json",
            ProjectRoot::Local(_) => "~/.claude/settings.json",
        };
        let message = match serde_json::to_string_pretty(&config) {
            Ok(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                format!("Claude Code の hook の設定をコピーしました。{place} の \"hooks\" に貼り付けてください。")
            }
            Err(e) => format!("hook の設定を作れません: {e}"),
        };
        self.show_error(message, window, cx);
    }

    /// 選択中のペインのターミナルに検索の欄を出す。
    fn find_in_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.active_project().and_then(|p| p.active_tab()).map(|t| t.panes[t.focused].id) else { return };
        let Some((view, _)) = self.terminals.get(&id) else { return };
        self.focused_pane = Some(id);
        view.update(cx, |v, cx| v.open_search(window, cx));
    }

    /// 次の描画で、選択中のペインへフォーカスを当て直す。
    pub(crate) fn refocus_pane(&mut self) {
        self.focused_pane = None;
    }

    /// root は正規化済みとする。shell は新しく開くときの既定のシェルの上書き。
    pub(crate) fn open_project(&mut self, root: ProjectRoot, shell: Option<ShellKind>, cx: &mut Context<Self>) {
        self.touch_recent(&root, cx);
        self.workspace.open_project_with(root, shell, OpenMode::WithTerminal);
        self.refresh_active_branch(cx);
    }

    /// 2 つ目の起動から届いた要求。Open ならプロジェクトを開いて（開いていれば切り替えて）、どちらも前面に出す。
    pub(crate) fn handle_message(&mut self, msg: Message, window: &mut Window, cx: &mut Context<Self>) {
        match msg {
            Message::Open { root, shell } => self.open_project(root, shell, cx),
            Message::OpenFile { file, root_hint } => self.open_file(file, root_hint, None, window, cx),
            Message::Activate => {}
            // hook の信号は前面に出さない。
            Message::Hook { terminal, signal } => {
                self.on_hook(&terminal, signal, window, cx);
                cx.notify();
                return;
            }
        }
        window.activate_window();
        cx.activate(true);
        cx.notify();
    }

    /// 今の状態が前回の保存と違えば、SAVE_DELAY のあとに書く。
    fn schedule_save(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(path) = self.session_path.clone() else { return };
        let geom = geom_from_bounds(window.window_bounds());
        self.last_geom = Some(geom.clone());
        let data = SessionData::capture_keeping(&self.workspace, Some(geom), self.vertical_tabs, self.last_session.as_ref());
        if data.has_projects() {
            self.last_session = Some(data.clone());
        }
        if self.saved.as_ref() == Some(&data) || self.pending.as_ref() == Some(&data) {
            return;
        }
        self.pending = Some(data.clone());
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let written = data.clone();
            let result = cx.background_spawn(async move { session::save(&path, &written) }).await;
            if let Err(e) = result {
                eprintln!("セッションを保存できない: {e}");
                return;
            }
            this.update(cx, |this, _| {
                this.saved = Some(data);
                this.pending = None;
            })
            .ok();
        }));
    }

    /// すぐに書く（ウィンドウを閉じるとき、アプリの終了時）。
    pub(crate) fn save_now(&mut self) {
        let Some(path) = self.session_path.as_ref() else { return };
        self.save_task = None;
        self.pending = None;
        let data = SessionData::capture_keeping(&self.workspace, self.last_geom.clone(), self.vertical_tabs, self.last_session.as_ref());
        if self.saved.as_ref() == Some(&data) {
            return;
        }
        match session::save(path, &data) {
            Ok(()) => self.saved = Some(data),
            Err(e) => eprintln!("セッションを保存できない: {e}"),
        }
    }

    /// 選択中のプロジェクトの既定のシェルでタブを作る。
    pub(crate) fn new_default_tab(&mut self) {
        if let Some(kind) = self.workspace.active_project().map(|p| self.workspace.new_terminal_shell(p)) {
            self.workspace.new_tab(kind);
        }
    }

    /// 選択中のプロジェクトのブランチ名を読み直す。WSL のプロジェクトは裏で読み、届いたら描き直す。
    /// 読めなければ None にし、理由を出す（次にそのプロジェクトを選んだときにやり直す）。
    pub(crate) fn refresh_active_branch(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.workspace.active_project().map(|p| p.id) {
            self.refresh_branch(id, cx);
        }
    }

    /// project のブランチ名を読み直す。WSL のプロジェクトは裏で読み、届いたら描き直す。
    pub(crate) fn refresh_branch(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let Some(p) = self.workspace.projects().iter().find(|p| p.id == project) else { return };
        let id = p.id;
        let root = p.root.clone();
        let read = move |hosts: &WslHosts| {
            project_fs(&root, hosts).and_then(|fs| fs.branch()).unwrap_or_else(|e| {
                eprintln!("ブランチ名を読めない: {}: {e}", root.copy_text());
                None
            })
        };
        match branch_read(&p.root) {
            BranchRead::Now => {
                self.branches.insert(id, read(&self.wsl_hosts));
            }
            BranchRead::Background => {
                let hosts = self.wsl_hosts.clone();
                let task = cx.background_spawn(async move { read(&hosts) });
                cx.spawn(async move |this, cx| {
                    let branch = task.await;
                    this.update(cx, |this, cx| {
                        this.branches.insert(id, branch);
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    /// フォルダを選ばせて、プロジェクトとして開く。選ばなかったときは何もしない。
    pub(crate) fn prompt_open_project(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let root = match std::fs::canonicalize(&path) {
                Ok(root) => ProjectRoot::from_host_path(fude_core::paths::simplify_verbatim(root)),
                Err(err) => {
                    eprintln!("フォルダを開けない: {}: {err}", path.display());
                    return;
                }
            };
            this.update(cx, |this, cx| {
                this.open_project(root, None, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// ホバーの出入りを HoverFades に伝える。
    pub(crate) fn hoverable(&self, el: Stateful<Div>, key: HoverKey, cx: &mut Context<Self>) -> Stateful<Div> {
        el.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.hover.set(key, *hovered, Instant::now());
            cx.notify();
        }))
    }

    /// base の不透明度に、ホバーの進み具合を掛けた色。
    pub(crate) fn hover_bg(&self, key: HoverKey, base: Rgba, now: Instant) -> Rgba {
        Rgba { a: base.a * self.hover.value(key, now), ..base }
    }

    /// 選択中のタブのペインにターミナルを用意し、無くなったペインのターミナルを捨てる。
    /// 選択中のペインが変わったら、そのターミナルにフォーカスを移す。
    fn sync_terminals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let alive: std::collections::HashSet<PaneId> =
            self.workspace.projects().iter().flat_map(|p| &p.tabs).flat_map(|t| &t.panes).map(|p| p.id).collect();
        self.terminals.retain(|id, _| alive.contains(id));
        self.pending_input.retain(|id, _| alive.contains(id));
        self.agents.retain(|raw| alive.iter().any(|p| p.raw() == raw));

        // フォーカスしていたビューが無くなった（タブやペインを閉じた）か。
        let lost = window.focused(cx).is_none();
        let Some(project) = self.workspace.active_project() else {
            // 最後のプロジェクトを閉じたときも、ビューが無くなるので、ワークスペース自身に戻しておく。
            if self.focused_pane.take().is_some() || lost {
                window.focus(&self.focus, cx);
            }
            return;
        };
        let root = project.root.clone();
        let Some(tab) = project.active_tab() else {
            // 最後のタブを閉じると、フォーカスしていたビューが無くなる。そのままだと、次に作った
            // タブのターミナルへフォーカスが移らないため、右パネル（無ければワークスペース自身）に戻しておく。
            if self.focused_pane.take().is_some() || lost {
                match self.active_editor_view() {
                    Some(view) => {
                        let focus = view.read(cx).focus_handle();
                        window.focus(&focus, cx);
                    }
                    None => window.focus(&self.focus, cx),
                }
            }
            return;
        };
        let panes: Vec<(PaneId, ShellKind)> = tab.panes.iter().map(|p| (p.id, p.kind.clone())).collect();
        let target = tab.panes[tab.focused].id;
        for (id, kind) in panes {
            if self.terminals.contains_key(&id) {
                continue;
            }
            let terminal_id = terminal_id(id);
            let view = cx.new(|cx| TerminalView::new(&kind, &root, &terminal_id, window, cx));
            let settings = self.settings.clone();
            view.update(cx, |v, cx| v.apply_settings(&settings, window, cx));
            let link_kind = kind.clone();
            let link_root = root.clone();
            let sub = cx.subscribe_in(&view, window, move |this, view, event: &TerminalEvent, window, cx| {
                match event {
                    TerminalEvent::OpenLink(target) => this.open_link(target.clone(), &link_kind, &link_root, window, cx),
                    TerminalEvent::Title(title) => this.workspace.set_pane_title(id, title.clone()),
                    TerminalEvent::Exited { success: true } => {
                        this.workspace.close_pane(id);
                    }
                    TerminalEvent::Exited { success: false } => {}
                    TerminalEvent::Input => {
                        this.agents.typed(id.raw());
                    }
                    TerminalEvent::DroppedFiles(files) => {
                        let text = fude_core::drop_text::drop_text(files, &link_kind, Platform::current());
                        view.read(cx).write(text.into_bytes());
                        this.agents.typed(id.raw());
                        this.workspace.focus_pane(id);
                        this.refocus_pane();
                    }
                    TerminalEvent::Focused => {
                        this.workspace.focus_pane(id);
                        this.focused_pane = Some(id);
                    }
                }
                cx.notify();
            });
            if let Some(input) = self.pending_input.remove(&id) {
                view.read(cx).write(input);
            }
            self.terminals.insert(id, (view, sub));
        }
        if self.focused_pane != Some(target)
            && let Some((view, _)) = self.terminals.get(&target)
        {
            self.focused_pane = Some(target);
            let handle = view.read(cx).focus_handle();
            window.focus(&handle, cx);
        }
    }

    /// ターミナルの出力のリンクを開く。URL はブラウザで、パスは右パネルで（見つからなければ何もしない）。
    fn open_link(
        &mut self,
        target: fude_term::links::LinkTarget,
        kind: &ShellKind,
        root: &ProjectRoot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use fude_core::location::FileLocation;
        use fude_term::links::{LinkTarget, resolve_link};
        match target {
            LinkTarget::Url(url) => cx.open_url(&url),
            LinkTarget::Path { path, line, col } => {
                let Some(file) = resolve_link(&path, kind, root) else { return };
                // ローカルはその場で確かめる。WSL は確かめずに開く（無ければ右パネルが「見つからない」と出す）。
                if let FileLocation::Local(p) = &file
                    && !p.is_file()
                {
                    return;
                }
                self.open_file(file, None, line.map(|l| (l, col)), window, cx);
            }
        }
    }

    /// タブの切り替え。エディタ列にフォーカスがあればファイルのタブ、それ以外はターミナルのタブ。
    fn cycle_focused_tabs(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let in_editor = self.active_editor_focus(cx).is_some_and(|f| f.contains_focused(window, cx));
        match self.workspace.active_project().map(|p| p.id) {
            Some(project) if in_editor => {
                if let Some(tabs) = self.workspace.editor_mut(project) {
                    tabs.cycle(delta);
                }
                self.show_active_tab(project, window, cx);
            }
            _ => self.workspace.cycle_tab(delta),
        }
        cx.notify();
    }

    /// ターミナルの大きさが変わる動き（分割、エディタ列、ツリー、エディタの移動）の途中か。
    pub(crate) fn layout_animating(&self, now: Instant) -> bool {
        self.anims.split_animating(now)
            || self.editor_animating(now)
            || self.tree_presence.is_animating(now)
            || self.pins.presence.is_animating(now)
            || self.vtabs_w.is_animating(now)
    }

    /// 動きの間は、選択中のタブのターミナルが PTY にサイズを送らないようにする。
    fn freeze_terminals(&mut self, frozen: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace.active_project().and_then(|p| p.active_tab()) else { return };
        let views: Vec<Entity<TerminalView>> =
            tab.panes.iter().filter_map(|p| self.terminals.get(&p.id).map(|(v, _)| v.clone())).collect();
        for view in views {
            view.update(cx, |v, cx| v.set_resize_frozen(frozen, cx));
        }
    }

    /// 表示したことのあるターミナル。
    pub(crate) fn terminal_views(&self) -> impl Iterator<Item = (PaneId, &Entity<TerminalView>)> {
        self.terminals.iter().map(|(id, (view, _))| (*id, view))
    }

    pub(crate) fn terminal_view(&self, pane: PaneId) -> Option<&Entity<TerminalView>> {
        self.terminals.get(&pane).map(|(view, _)| view)
    }

    fn activate_project_at(&mut self, index: usize, cx: &mut Context<Self>) {
        self.workspace.activate_project_at(index);
        self.refresh_active_branch(cx);
        cx.notify();
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        // カーソルがウィンドウの外へ出ると、GPUI は再描画するが、部品の on_hover(false) は呼ばない。
        // ウィンドウ全体のホバーの濃さで表示から消す（HoverFades の説明を参照）。
        self.hover.set_window_hovered(window.is_window_hovered(), now);
        if self.hover.any_animating(now) {
            window.request_animation_frame();
        }
        let theme = *cx.global::<Theme>();
        let (bg, fg) = (theme.bg_window, theme.fg_default);
        self.sync_terminals(window, cx);
        self.sync_live(cx);
        self.sync_pins(now, window, cx);
        self.ensure_active_view(window, cx);
        self.anims.observe(self.workspace.active_project(), now, self.reduced_motion);
        let editor_shape =
            self.workspace.active_project().map(|p| (p.id, p.editor.items().iter().map(|i| i.rel.clone()).collect()));
        self.anims.observe_editor(editor_shape, now, self.reduced_motion);
        self.observe_editor_column(now);
        self.observe_tree(now);
        if self.anims.tabs_animating(now)
            || self.anims.editor_tabs_animating(now)
            || self.editor_animating(now)
            || self.tree_presence.is_animating(now)
            || self.pins.presence.is_animating(now)
        {
            window.request_animation_frame();
        }
        self.freeze_terminals(self.layout_animating(now), cx);
        self.schedule_save(window, cx);
        self.observe_waiting(window, now);
        self.observe_route_map(window, now, cx);
        self.observe_vertical_tabs(window, now);
        let home = self.workspace.projects().is_empty();
        if self.showing_home != Some(home) {
            // 最初の描画ではフェードしない。
            if self.showing_home.is_some() {
                self.screen_fade = Fade::settled(0.0);
                self.screen_fade.animate_to(1.0, SCREEN_FADE, ease_out, now);
            }
            if home {
                self.check_missing_roots(cx);
            }
            self.showing_home = Some(home);
        }
        if self.screen_fade.is_animating(now) {
            window.request_animation_frame();
        }
        let main = if home {
            self.render_home(cx, now)
        } else {
            let terminals = self.terminals.iter().map(|(id, (view, _))| (*id, view.clone())).collect();
            let main_w = window.viewport_size().width
                - crate::theme::size::RAIL_W
                - px(self.route_map_width(now))
                - px(self.vertical_tabs_width(now));
            let (tree, editor, pin) = self.render_columns(main_w, window.viewport_size().width, now, cx);
            let active_tab = self.workspace.active_project().and_then(|p| p.active_tab());
            let share = active_tab.map_or(1.0, |t| self.anims.split_share(t.id, now));
            render_main_area(active_tab, &terminals, share, tree, editor, pin, cx)
        };
        let main = main.opacity(self.screen_fade.value(now));
        let title_row = self.render_title_row(window, cx, now);
        let rail = self.render_rail(cx, now);
        let route_map = self.render_route_map(now, cx);
        let vertical_tabs = self.render_vertical_tabs(now, cx);
        let menu = self.render_shell_menu(window, cx, now);
        let project_menu = self.render_project_menu(window, cx, now);
        let home_menu = self.render_home_menu(window, cx, now);
        let changes_menu = self.render_changes_menu(window, cx, now);
        let tree_menu = self.render_tree_menu(window, cx, now);
        let dialog = self.render_dialog(window, cx, now);
        let palette = self.render_palette(window, cx, now);
        let wait_frame = self.render_wait_frame(&theme, now);
        let config_notice = self.render_config_notice(&theme, cx);

        div()
            .id("workspace")
            .track_focus(&self.focus)
            .key_context("Workspace")
            .on_action(cx.listener(|this, _: &crate::tree_view::ToggleTree, window, cx| this.toggle_tree(window, cx)))
            .on_action(cx.listener(|this, _: &CloseItem, window, cx| this.close_key(window, cx)))
            .on_action(cx.listener(|this, _: &OpenPalette, window, cx| {
                this.open_palette(fude_core::palette::Scope::All, "", window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToFile, window, cx| {
                this.open_palette(fude_core::palette::Scope::Files, "", window, cx)
            }))
            .on_action(cx.listener(|this, _: &FindInFiles, window, cx| this.open_find_in_files(window, cx)))
            .on_action(cx.listener(|this, _: &FindInTerminal, window, cx| this.find_in_terminal(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleRouteMap, _, cx| this.toggle_route_map(cx)))
            .on_action(cx.listener(|this, _: &NewTab, _, cx| {
                this.new_default_tab();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SplitTab, _, cx| {
                this.workspace.split_active_tab();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle_focused_tabs(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.cycle_focused_tabs(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject1, _, cx| this.activate_project_at(0, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject2, _, cx| this.activate_project_at(1, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject3, _, cx| this.activate_project_at(2, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject4, _, cx| this.activate_project_at(3, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject5, _, cx| this.activate_project_at(4, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject6, _, cx| this.activate_project_at(5, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject7, _, cx| this.activate_project_at(6, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject8, _, cx| this.activate_project_at(7, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject9, _, cx| this.activate_project_at(8, cx)))
            // ターミナルとエディタの上以外に落とされたファイルとフォルダを開く（設計書 §5.1）。
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.open_dropped(paths, window, cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .text_color(fg)
            .font(ui_font())
            .child(title_row)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(rail)
                    .children(route_map)
                    .children(vertical_tabs)
                    .child(div().flex_1().min_w_0().h_full().child(main)),
            )
            .children(config_notice)
            .children(menu)
            .children(project_menu)
            .children(home_menu)
            .children(changes_menu)
            .children(tree_menu)
            .children(palette)
            .children(dialog)
            .children(wait_frame)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn key_notation_is_checked() {
        use super::valid_key;
        use gpui_kit::OperatingSystem::Windows;
        assert!(valid_key("ctrl-shift-p", Windows));
        assert!(valid_key("cmd-k cmd-s", Windows));
        assert!(!valid_key("", Windows));
        assert!(!valid_key("   ", Windows));
    }

    #[test]
    fn bad_keys_become_issues() {
        use super::valid_bindings;
        use fude_core::settings::Settings;
        let mut s = Settings::default();
        let row = vec![("close".to_string(), vec!["".to_string(), "ctrl-q".to_string()])];
        s.keybindings.windows = row.clone();
        s.keybindings.macos = row;
        let (list, issues) = valid_bindings(&s);
        assert_eq!(issues.len(), 1);
        assert!(list.iter().any(|b| b.key == "ctrl-q"));
        assert!(!list.iter().any(|b| b.key.is_empty()));
    }

    use super::geom_from_bounds;
    use fude_core::session::WindowGeom;
    use gpui_kit::{Bounds, WindowBounds, point, px, size};

    #[test]
    fn hook_command_per_root() {
        use super::hook_command_for;
        use fude_core::claude::HookCommand;
        use fude_core::workspace::ProjectRoot;
        use std::path::{Path, PathBuf};
        let exe = Path::new(r"C:\Program Files\fude\fude.exe");
        let local = hook_command_for(&ProjectRoot::Local(PathBuf::from(r"C:\w")), exe, "~/.local/bin/fude");
        assert!(matches!(local, HookCommand::Exec(ref p) if p == "C:/Program Files/fude/fude.exe"));
        let wsl = hook_command_for(&ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x".into() }, exe, "~/.local/bin/fude");
        assert!(matches!(wsl, HookCommand::Shell(ref s) if s == "~/.local/bin/fude"));
    }

    #[test]
    fn claude_launch_input_runs_claude() {
        assert_eq!(super::CLAUDE_LAUNCH_INPUT, b"claude\r");
    }

    #[test]
    fn last_project_closed_rules() {
        use super::{AfterClose, last_project_closed_action};
        use fude_core::settings::{LastProjectClosed, Settings};
        let home = Settings::default();
        let mut quit = Settings::default();
        quit.close.on_last_project = LastProjectClosed::Quit;
        assert_eq!(last_project_closed_action(&home, 1), AfterClose::Stay);
        assert_eq!(last_project_closed_action(&quit, 2), AfterClose::Stay);
        assert_eq!(last_project_closed_action(&home, 0), AfterClose::ShowHome);
        assert_eq!(last_project_closed_action(&quit, 0), AfterClose::Quit);
    }

    #[test]
    fn wsl_branch_is_read_in_background() {
        use super::{BranchRead, branch_read};
        use fude_core::workspace::ProjectRoot;
        let wsl = ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x".into() };
        assert_eq!(branch_read(&wsl), BranchRead::Background);
        let local = ProjectRoot::Local(std::path::PathBuf::from("/w/a"));
        assert_eq!(branch_read(&local), BranchRead::Now);
    }

    #[test]
    fn geom_from_bounds_maps_states() {
        let b = Bounds::new(point(px(10.), px(20.)), size(px(1200.), px(780.)));
        let windowed = WindowGeom { x: 10., y: 20., width: 1200., height: 780., maximized: false };
        assert_eq!(geom_from_bounds(WindowBounds::Windowed(b)), windowed);
        assert_eq!(geom_from_bounds(WindowBounds::Maximized(b)), WindowGeom { maximized: true, ..windowed.clone() });
        assert_eq!(geom_from_bounds(WindowBounds::Fullscreen(b)), WindowGeom { maximized: true, ..windowed });
    }
}
