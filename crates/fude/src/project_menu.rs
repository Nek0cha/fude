//! レールのアイコンの右クリックで出す、プロジェクトのメニュー。

use std::time::Instant;

use fude_core::keymap::Platform;
use fude_core::workspace::{ProjectId, ProjectRoot};
use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::hover::HoverKey;
use crate::popup::{ITEM_H, Popup, place};
use crate::theme::{Theme, size};
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

/// ＋のメニューと HoverKey::MenuItem が重ならないように、項目の番号をずらす。
const HOVER_OFFSET: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ProjectAction {
    Close,
    Reveal,
    CopyPath,
}

/// 並びは設計書 §4.3 の順（閉じる、エクスプローラー／Finder で開く、パスをコピー）。
const ACTIONS: [ProjectAction; 3] = [ProjectAction::Close, ProjectAction::Reveal, ProjectAction::CopyPath];

/// 表示する文字。
pub(crate) fn action_label(action: ProjectAction, os: Platform) -> &'static str {
    match (action, os) {
        (ProjectAction::Close, _) => "閉じる",
        (ProjectAction::Reveal, Platform::Windows) => "エクスプローラーで開く",
        (ProjectAction::Reveal, Platform::MacOs) => "Finder で表示",
        (ProjectAction::CopyPath, _) => "パスをコピー",
    }
}

fn action_icon(action: ProjectAction) -> IconName {
    match action {
        ProjectAction::Close => IconName::X,
        ProjectAction::Reveal => IconName::FolderOpen,
        ProjectAction::CopyPath => IconName::Copy,
    }
}

/// 開くためのコマンド。Windows は ("explorer.exe", [host_path])、macOS は ("open", ["-R", host_path])。
pub(crate) fn reveal_command(root: &ProjectRoot, os: Platform) -> (String, Vec<String>) {
    let path = root.host_path().display().to_string();
    match os {
        Platform::Windows => ("explorer.exe".to_string(), vec![path]),
        Platform::MacOs => ("open".to_string(), vec!["-R".to_string(), path]),
    }
}

impl WorkspaceView {
    /// project のメニューを、押した位置を左上にして出す。選択中のプロジェクトは変えない。
    pub(crate) fn open_project_menu(
        &mut self,
        project: ProjectId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_menu = Some((project, Popup::open(position, window, cx)));
        cx.notify();
    }

    pub(crate) fn close_project_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project_menu.as_mut().is_some_and(|(_, m)| m.close(Instant::now())) {
            self.restore_focus(window, cx);
            cx.notify();
        }
    }

    fn run_project_action(&mut self, project: ProjectId, action: ProjectAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.projects().iter().find(|p| p.id == project).map(|p| p.root.clone()) else {
            return;
        };
        match action {
            ProjectAction::Close => {
                self.workspace.close_project(project);
                self.branches.remove(&project);
                self.refresh_active_branch(cx);
            }
            ProjectAction::Reveal => {
                let (program, args) = reveal_command(&root, Platform::current());
                if let Err(e) = std::process::Command::new(&program).args(&args).spawn() {
                    eprintln!("{program} を起動できない: {e}");
                }
            }
            ProjectAction::CopyPath => cx.write_to_clipboard(ClipboardItem::new_string(root.copy_text())),
        }
        self.close_project_menu(window, cx);
    }

    pub(crate) fn render_project_menu(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Option<Deferred> {
        let (project, menu) = self.project_menu.as_ref()?;
        let project = *project;
        if menu.is_gone(now) {
            self.project_menu = None;
            return None;
        }
        if menu.is_animating(now) {
            window.request_animation_frame();
        }
        let theme = *cx.global::<Theme>();
        let closing = menu.closing();
        let os = Platform::current();
        let items: Vec<AnyElement> = ACTIONS
            .iter()
            .enumerate()
            .map(|(i, &action)| {
                let key = HoverKey::MenuItem(HOVER_OFFSET + i);
                let bg = self.hover_bg(key, theme.bg_hover, now);
                self.hoverable(div().id(ElementId::Name(format!("project-menu-{i}").into())), key, cx)
                    .h(ITEM_H)
                    .px(px(8.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .rounded(size::RADIUS_SM)
                    .bg(bg)
                    .child(icon(action_icon(action), theme.fg_dim))
                    .child(action_label(action, os))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.project_menu.as_ref().is_some_and(|(_, m)| m.closing()) {
                            return;
                        }
                        this.run_project_action(project, action, window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let body = menu
            .frame("project-menu", &theme, now)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_project_menu(window, cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_project_menu(window, cx)))
            .children(if closing { Vec::new() } else { items });
        Some(place(menu.position(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::{ProjectAction, action_label, reveal_command};
    use fude_core::keymap::Platform;
    use fude_core::workspace::ProjectRoot;
    use std::path::PathBuf;

    #[test]
    fn reveal_command_per_os() {
        let wsl = ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x".into() };
        assert_eq!(
            reveal_command(&wsl, Platform::Windows),
            ("explorer.exe".to_string(), vec![r"\\wsl.localhost\Ubuntu\home\x".to_string()])
        );
        let mac = ProjectRoot::Local(PathBuf::from("/Users/u/p"));
        assert_eq!(
            reveal_command(&mac, Platform::MacOs),
            ("open".to_string(), vec!["-R".to_string(), "/Users/u/p".to_string()])
        );
    }

    #[test]
    fn labels_per_os() {
        assert_ne!(action_label(ProjectAction::Reveal, Platform::Windows), action_label(ProjectAction::Reveal, Platform::MacOs));
        for a in [ProjectAction::Close, ProjectAction::CopyPath] {
            assert_eq!(action_label(a, Platform::Windows), action_label(a, Platform::MacOs));
        }
    }
}
