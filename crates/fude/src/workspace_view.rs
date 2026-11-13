//! ルートのビュー。全体の配置と、アクションの処理を受け持つ。

use std::collections::HashMap;
use std::time::Instant;

use fude_core::git_head::read_branch;
use fude_core::keymap::{Command, Platform, default_bindings};
use fude_core::workspace::{PaneId, ProjectId, ProjectRoot, ShellKind, Workspace};
use gpui_kit::*;

use crate::hover::{HoverFades, HoverKey};
use crate::main_area::render_main_area;
use crate::shell_menu::ShellMenu;
use crate::terminal_view::{TerminalEvent, TerminalView};
use crate::theme::{Theme, ui_font};

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
    ]
);

/// fude_core の割り当て表を GPUI のキーバインドに変換して登録する。
pub fn bind_keys(cx: &mut App) {
    let bindings = default_bindings(Platform::current()).into_iter().map(|(key, command)| match command {
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
    });
    cx.bind_keys(bindings.collect::<Vec<_>>());
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
    pub(crate) shell_menu: Option<ShellMenu>,
    /// インストール済みの WSL のディストリビューション。起動時に 1 回だけ読む。
    pub(crate) distros: Vec<String>,
    focus: FocusHandle,
    _activation: Subscription,
}

impl WorkspaceView {
    pub fn new(initial_root: ProjectRoot, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh_active_branch();
                cx.notify();
            }
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            workspace: Workspace::default(),
            branches: HashMap::new(),
            hover: HoverFades::default(),
            should_move: false,
            terminals: HashMap::new(),
            focused_pane: None,
            shell_menu: None,
            distros: Vec::new(),
            focus,
            _activation: activation,
        };
        this.open_project(initial_root);
        this.load_distros(cx);
        this
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

    /// 次の描画で、選択中のペインへフォーカスを当て直す。
    pub(crate) fn refocus_pane(&mut self) {
        self.focused_pane = None;
    }

    /// root は正規化済みとする。
    pub(crate) fn open_project(&mut self, root: ProjectRoot) {
        let id = self.workspace.open_project(root);
        let branch = self.workspace.active_project().and_then(|p| read_branch(&p.root.host_path()));
        self.branches.insert(id, branch);
    }

    /// 選択中のプロジェクトの既定のシェルでタブを作る。
    pub(crate) fn new_default_tab(&mut self) {
        if let Some(kind) = self.workspace.active_project().map(|p| p.default_shell()) {
            self.workspace.new_tab(kind);
        }
    }

    fn refresh_active_branch(&mut self) {
        if let Some(p) = self.workspace.active_project() {
            self.branches.insert(p.id, read_branch(&p.root.host_path()));
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
                this.open_project(root);
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

        let Some(project) = self.workspace.active_project() else { return };
        let root = project.root.clone();
        let Some(tab) = project.active_tab() else {
            // 最後のタブを閉じると、フォーカスしていたビューが無くなる。そのままだと、次に作った
            // タブのターミナルへフォーカスが移らないため、ワークスペース自身に戻しておく。
            if self.focused_pane.take().is_some() {
                window.focus(&self.focus, cx);
            }
            return;
        };
        let panes: Vec<(PaneId, ShellKind)> = tab.panes.iter().map(|p| (p.id, p.kind.clone())).collect();
        let target = tab.panes[tab.focused].id;
        for (id, kind) in panes {
            if self.terminals.contains_key(&id) {
                continue;
            }
            let view = cx.new(|cx| TerminalView::new(&kind, &root, window, cx));
            let sub = cx.subscribe(&view, move |this, _, event: &TerminalEvent, cx| {
                match event {
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

    fn activate_project_at(&mut self, index: usize, cx: &mut Context<Self>) {
        self.workspace.activate_project_at(index);
        self.refresh_active_branch();
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
        let active_tab = self.workspace.active_project().and_then(|p| p.active_tab());
        let terminals = self.terminals.iter().map(|(id, (view, _))| (*id, view.clone())).collect();
        let main = render_main_area(active_tab, &terminals, cx);
        let title_row = self.render_title_row(window, cx, now);
        let rail = self.render_rail(cx, now);
        let menu = self.render_shell_menu(window, cx, now);

        div()
            .id("workspace")
            .track_focus(&self.focus)
            .key_context("Workspace")
            .on_action(cx.listener(|this, _: &NewTab, _, cx| {
                this.new_default_tab();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SplitTab, _, cx| {
                this.workspace.split_active_tab();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                this.workspace.cycle_tab(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| {
                this.workspace.cycle_tab(-1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ActivateProject1, _, cx| this.activate_project_at(0, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject2, _, cx| this.activate_project_at(1, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject3, _, cx| this.activate_project_at(2, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject4, _, cx| this.activate_project_at(3, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject5, _, cx| this.activate_project_at(4, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject6, _, cx| this.activate_project_at(5, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject7, _, cx| this.activate_project_at(6, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject8, _, cx| this.activate_project_at(7, cx)))
            .on_action(cx.listener(|this, _: &ActivateProject9, _, cx| this.activate_project_at(8, cx)))
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
                    .child(rail)
                    .child(div().flex_1().min_w_0().h_full().child(main)),
            )
            .children(menu)
    }
}
