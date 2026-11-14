//! ルートのビュー。全体の配置と、アクションの処理を受け持つ。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fude_core::keymap::{Command, Platform, context_bindings, default_bindings};
use fude_core::recent::RecentList;
use fude_core::session::{self, SessionData, WindowGeom};
use fude_core::settings::{LastProjectClosed, Settings};
use fude_core::workspace::{OpenMode, PaneId, ProjectId, ProjectRoot, ShellKind, Workspace};
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
    ]
);

/// Command と文脈を GPUI のキーバインドにする。
fn binding(key: &'static str, command: Command, context: Option<&'static str>) -> KeyBinding {
    use crate::file_view::SaveFile;
    use crate::tree_view::ToggleTree;
    match command {
        Command::Close => KeyBinding::new(key, CloseItem, context),
        Command::ToggleTree => KeyBinding::new(key, ToggleTree, context),
        Command::Save => KeyBinding::new(key, SaveFile, context),
        Command::OpenPalette => KeyBinding::new(key, OpenPalette, context),
        Command::GoToFile => KeyBinding::new(key, GoToFile, context),
        Command::FindInFiles => KeyBinding::new(key, FindInFiles, context),
        Command::FindInTerminal => KeyBinding::new(key, FindInTerminal, context),
        other => default_binding(key, other),
    }
}

/// fude_core の割り当て表を GPUI のキーバインドに変換して登録する。
pub fn bind_keys(cx: &mut App) {
    let platform = Platform::current();
    let mut bindings: Vec<KeyBinding> =
        default_bindings(platform).into_iter().map(|(key, command)| binding(key, command, None)).collect();
    bindings.extend(context_bindings(platform).into_iter().map(|(key, command, context)| binding(key, command, Some(context))));
    cx.bind_keys(bindings);
}

fn default_binding(key: &'static str, command: Command) -> KeyBinding {
    match command {
        Command::NewTab => KeyBinding::new(key, NewTab, None),
        Command::SplitTab => KeyBinding::new(key, SplitTab, None),
        Command::NextTab => KeyBinding::new(key, NextTab, None),
        Command::PrevTab => KeyBinding::new(key, PrevTab, None),
        Command::Copy => KeyBinding::new(key, CopySelection, Some("Terminal")),
        Command::Paste => KeyBinding::new(key, PasteText, Some("Terminal")),
        Command::ActivateProject(n) => match n {
            1 => KeyBinding::new(key, ActivateProject1, None),
            2 => KeyBinding::new(key, ActivateProject2, None),
            3 => KeyBinding::new(key, ActivateProject3, None),
            4 => KeyBinding::new(key, ActivateProject4, None),
            5 => KeyBinding::new(key, ActivateProject5, None),
            6 => KeyBinding::new(key, ActivateProject6, None),
            7 => KeyBinding::new(key, ActivateProject7, None),
            8 => KeyBinding::new(key, ActivateProject8, None),
            _ => KeyBinding::new(key, ActivateProject9, None),
        },
        // 文脈の付くものは binding で扱う。
        Command::Close
        | Command::ToggleTree
        | Command::Save
        | Command::OpenPalette
        | Command::GoToFile
        | Command::FindInFiles
        | Command::FindInTerminal => {
            unreachable!("binding で扱う")
        }
    }
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
    match (remaining, settings.on_last_project_closed) {
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
    /// ピル用のブランチ名。開いたときと、ウィンドウが前面に来たときに読み直す。
    pub(crate) branches: HashMap<ProjectId, Option<String>>,
    pub(crate) hover: HoverFades,
    /// macOS でタイトル行をドラッグしてウィンドウを動かすための状態。
    pub(crate) should_move: bool,
    /// ペインごとのターミナル。表示したときに作り、ペインが無くなったら捨てる（子プロセスも終わる）。
    terminals: HashMap<PaneId, (Entity<TerminalView>, Subscription)>,
    /// 最後にフォーカスを当てたペイン。選択が変わったら、新しいペインへフォーカスを移す。
    focused_pane: Option<PaneId>,
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
    /// 設定（段階 9 までは既定値）。
    pub(crate) settings: Settings,
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
            branches: HashMap::new(),
            hover: HoverFades::default(),
            should_move: false,
            terminals: HashMap::new(),
            focused_pane: None,
            shell_menu: None,
            project_menu: None,
            distros: Vec::new(),
            wsl_hosts: Arc::new(WslHosts::new()),
            focus,
            session_path,
            settings: Settings::default(),
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
        match open {
            InitialOpen::Project { root, shell } => this.open_project(root, shell, cx),
            InitialOpen::File { file, root_hint } => this.open_file(file, root_hint, None, window, cx),
            InitialOpen::Nothing => {}
        }
        this.refresh_active_branch(cx);
        this.load_distros(cx);
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
        let data = SessionData::capture_keeping(&self.workspace, Some(geom), self.last_session.as_ref());
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
        let data = SessionData::capture_keeping(&self.workspace, self.last_geom.clone(), self.last_session.as_ref());
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
        if let Some(kind) = self.workspace.active_project().map(|p| p.default_shell()) {
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
            let link_kind = kind.clone();
            let link_root = root.clone();
            let sub = cx.subscribe_in(&view, window, move |this, _, event: &TerminalEvent, window, cx| {
                match event {
                    TerminalEvent::OpenLink(target) => this.open_link(target.clone(), &link_kind, &link_root, window, cx),
                    TerminalEvent::Title(title) => this.workspace.set_pane_title(id, title.clone()),
                    TerminalEvent::Exited { success: true } => {
                        this.workspace.close_pane(id);
                    }
                    TerminalEvent::Exited { success: false } => {}
                    TerminalEvent::Focused => {
                        this.workspace.focus_pane(id);
                        this.focused_pane = Some(id);
                    }
                }
                cx.notify();
            });
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
        self.anims.split_animating(now) || self.editor_animating(now) || self.tree_presence.is_animating(now)
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
        {
            window.request_animation_frame();
        }
        self.freeze_terminals(self.layout_animating(now), cx);
        self.schedule_save(window, cx);
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
            let main_w = window.viewport_size().width - crate::theme::size::RAIL_W;
            let (tree, editor) = self.render_columns(main_w, now, cx);
            let active_tab = self.workspace.active_project().and_then(|p| p.active_tab());
            let share = active_tab.map_or(1.0, |t| self.anims.split_share(t.id, now));
            render_main_area(active_tab, &terminals, share, tree, editor, cx)
        };
        let main = main.opacity(self.screen_fade.value(now));
        let title_row = self.render_title_row(window, cx, now);
        let rail = self.render_rail(cx, now);
        let menu = self.render_shell_menu(window, cx, now);
        let project_menu = self.render_project_menu(window, cx, now);
        let home_menu = self.render_home_menu(window, cx, now);
        let changes_menu = self.render_changes_menu(window, cx, now);
        let tree_menu = self.render_tree_menu(window, cx, now);
        let dialog = self.render_dialog(window, cx, now);
        let palette = self.render_palette(window, cx, now);

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
                    .child(div().flex_1().min_w_0().h_full().child(main)),
            )
            .children(menu)
            .children(project_menu)
            .children(home_menu)
            .children(changes_menu)
            .children(tree_menu)
            .children(palette)
            .children(dialog)
    }
}

#[cfg(test)]
mod tests {
    use super::geom_from_bounds;
    use fude_core::session::WindowGeom;
    use gpui_kit::{Bounds, WindowBounds, point, px, size};

    #[test]
    fn last_project_closed_rules() {
        use super::{AfterClose, last_project_closed_action};
        use fude_core::settings::{LastProjectClosed, Settings};
        let home = Settings::default();
        let quit = Settings { on_last_project_closed: LastProjectClosed::Quit, ..Settings::default() };
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
