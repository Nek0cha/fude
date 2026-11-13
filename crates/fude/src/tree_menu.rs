//! ツリーの右クリックメニュー（設計書 §4.3。「Claude に渡す」は計画7）。
//! 新規と名前の変更は、ツリーの上部に 1 行の入力欄を出し、Enter で確定、Esc で取り消す。

use std::path::Path;
use std::time::Instant;

use fude_core::keymap::Platform;
use fude_core::workspace::ProjectId;
use fude_fs::RelPath;
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::dialog::{ButtonRole, Dialog, DialogButton};
use crate::hover::HoverKey;
use crate::popup::{ITEM_H, Popup, place};
use crate::theme::{Theme, size};
use crate::title_row::icon;
use crate::tree_model::TreeRow;
use crate::workspace_view::WorkspaceView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TreeAction {
    NewFile,
    NewFolder,
    Rename,
    Delete,
    CopyPath,
    Reveal,
}

/// 設計書 §4.3 の並び（「Claude に渡す」を除く）。
const ACTIONS: [TreeAction; 6] =
    [TreeAction::NewFile, TreeAction::NewFolder, TreeAction::Rename, TreeAction::Delete, TreeAction::CopyPath, TreeAction::Reveal];

fn label(action: TreeAction, os: Platform) -> &'static str {
    match (action, os) {
        (TreeAction::NewFile, _) => "新規ファイル",
        (TreeAction::NewFolder, _) => "新規フォルダ",
        (TreeAction::Rename, _) => "名前の変更",
        (TreeAction::Delete, _) => "削除",
        (TreeAction::CopyPath, _) => "パスをコピー",
        (TreeAction::Reveal, Platform::Windows) => "エクスプローラーで表示",
        (TreeAction::Reveal, Platform::MacOs) => "Finder で表示",
    }
}

fn action_icon(action: TreeAction) -> IconName {
    match action {
        TreeAction::NewFile => IconName::FilePlus,
        TreeAction::NewFolder => IconName::FolderPlus,
        TreeAction::Rename => IconName::Pencil,
        TreeAction::Delete => IconName::Trash,
        TreeAction::CopyPath => IconName::Copy,
        TreeAction::Reveal => IconName::FolderOpen,
    }
}

/// ファイル名として使えるか。空、区切り文字、"." と ".."、Windows で使えない文字を拒む。
pub(crate) fn validate_name(name: &str) -> Result<(), &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("名前が空");
    }
    if name == "." || name == ".." {
        return Err("使えない名前");
    }
    if name.chars().any(|c| matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control()) {
        return Err("使えない文字を含む");
    }
    Ok(())
}

/// 新規の作り先。フォルダの行ならその中、ファイルの行ならその隣、行が無ければルート。
pub(crate) fn new_item_parent(row: Option<&TreeRow>) -> RelPath {
    match row {
        Some(r) if r.is_dir => r.rel.clone(),
        Some(r) => r.rel.as_str().rsplit_once('/').and_then(|(p, _)| RelPath::new(p).ok()).unwrap_or_else(RelPath::root),
        None => RelPath::root(),
    }
}

/// ファイルをエクスプローラー／Finder で選んだ状態で開くコマンド。
fn reveal_file_command(path: &Path, os: Platform) -> (String, Vec<String>) {
    match os {
        Platform::Windows => ("explorer.exe".into(), vec![format!("/select,{}", path.display())]),
        Platform::MacOs => ("open".into(), vec!["-R".into(), path.display().to_string()]),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditKind {
    NewFile,
    NewFolder,
    Rename,
}

/// 入力欄で名前を入れている途中の操作。
pub(crate) struct TreeEdit {
    pub project: ProjectId,
    pub kind: EditKind,
    /// 新規は作り先のフォルダ、名前の変更は対象。
    pub target: RelPath,
    pub input: Entity<InputState>,
    _sub: Subscription,
}

/// メニューの HoverKey の番号。
const MENU_HOVER: usize = 400;

impl WorkspaceView {
    pub(crate) fn open_tree_menu(&mut self, row: Option<TreeRow>, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.tree.menu = Some((row, Popup::open(position, window, cx)));
        cx.notify();
    }

    fn close_tree_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.menu.as_mut().is_some_and(|(_, m)| m.close(Instant::now())) {
            window.focus(&self.tree.focus, cx);
            cx.notify();
        }
    }

    fn run_tree_action(&mut self, row: Option<TreeRow>, action: TreeAction, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tree_menu(window, cx);
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        match action {
            TreeAction::NewFile => self.start_tree_edit(project, EditKind::NewFile, new_item_parent(row.as_ref()), "", window, cx),
            TreeAction::NewFolder => self.start_tree_edit(project, EditKind::NewFolder, new_item_parent(row.as_ref()), "", window, cx),
            TreeAction::Rename => {
                if let Some(row) = row {
                    self.start_tree_edit(project, EditKind::Rename, row.rel.clone(), &row.name, window, cx);
                }
            }
            TreeAction::Delete => {
                if let Some(row) = row {
                    self.confirm_tree_delete(project, row, window, cx);
                }
            }
            TreeAction::CopyPath => {
                if let Some(row) = row {
                    cx.write_to_clipboard(ClipboardItem::new_string(row.rel.as_str().to_string()));
                }
            }
            TreeAction::Reveal => {
                let Some(root) = self.workspace.projects().iter().find(|p| p.id == project).map(|p| p.root.clone()) else {
                    return;
                };
                let rel = row.map(|r| r.rel).unwrap_or_else(RelPath::root);
                let (program, args) = reveal_file_command(&rel.to_path(&root.host_path()), Platform::current());
                if let Err(e) = std::process::Command::new(&program).args(&args).spawn() {
                    eprintln!("{program} を起動できない: {e}");
                }
            }
        }
    }

    fn start_tree_edit(
        &mut self,
        project: ProjectId,
        kind: EditKind,
        target: RelPath,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial = initial.to_string();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(initial, window, cx);
            state
        });
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.commit_tree_edit(window, cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.tree.edit = Some(TreeEdit { project, kind, target, input, _sub: sub });
        cx.notify();
    }

    pub(crate) fn cancel_tree_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.edit.take().is_some() {
            window.focus(&self.tree.focus, cx);
            cx.notify();
        }
    }

    fn commit_tree_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.tree.edit.as_ref() else { return };
        let name = edit.input.read(cx).value().trim().to_string();
        if let Err(why) = validate_name(&name) {
            eprintln!("{why}: {name}");
            return;
        }
        let (project, kind, target) = (edit.project, edit.kind, edit.target.clone());
        let Some(fs) = self.live_fs(project) else { return };
        let parent = match kind {
            EditKind::Rename => {
                target.as_str().rsplit_once('/').and_then(|(p, _)| RelPath::new(p).ok()).unwrap_or_else(RelPath::root)
            }
            _ => target.clone(),
        };
        let Ok(dest) = parent.join(&name) else { return };
        self.tree.edit = None;
        window.focus(&self.tree.focus, cx);
        let task = cx.background_spawn({
            let dest = dest.clone();
            async move {
                match kind {
                    EditKind::NewFile => fs.create_file(&dest),
                    EditKind::NewFolder => fs.create_dir(&dest),
                    EditKind::Rename => fs.rename(&target, &dest),
                }
            }
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            cx.update_window(window_handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    if let Err(e) = result {
                        eprintln!("ツリーの操作に失敗した: {}: {e}", dest.as_str());
                        return;
                    }
                    // 作り先のフォルダを開いて読み直す（監視の通知を待たない）。
                    this.tree.models.entry(project).or_default().expand(&parent);
                    this.load_tree_dir(project, parent.clone(), cx);
                    if kind == EditKind::NewFile {
                        this.open_in_panel(project, dest.as_str().to_string(), None, window, cx);
                    }
                })
                .ok();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn confirm_tree_delete(&mut self, project: ProjectId, row: TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = Dialog {
            message: format!("{} を削除しますか？元に戻せません。", row.name),
            buttons: vec![
                DialogButton { label: "キャンセル", role: ButtonRole::Cancel },
                DialogButton { label: "削除", role: ButtonRole::Danger },
            ],
            on_choose: Box::new(move |i, this, _, cx| {
                if i != 1 {
                    return;
                }
                let Some(fs) = this.live_fs(project) else { return };
                let rel = row.rel.clone();
                let task = cx.background_spawn(async move { fs.remove(&rel) });
                let parent =
                    row.rel.as_str().rsplit_once('/').and_then(|(p, _)| RelPath::new(p).ok()).unwrap_or_else(RelPath::root);
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    this.update(cx, |this, cx| match result {
                        Ok(()) => this.load_tree_dir(project, parent, cx),
                        Err(e) => eprintln!("削除できない: {}: {e}", row.rel.as_str()),
                    })
                    .ok();
                })
                .detach();
            }),
        };
        self.show_dialog(dialog, window, cx);
    }

    pub(crate) fn render_tree_menu(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Option<Deferred> {
        let (row, menu) = self.tree.menu.as_ref()?;
        let row = row.clone();
        if menu.is_gone(now) {
            self.tree.menu = None;
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
            .copied()
            .filter(|a| row.is_some() || matches!(a, TreeAction::NewFile | TreeAction::NewFolder | TreeAction::Reveal))
            .enumerate()
            .map(|(i, action)| {
                let key = HoverKey::MenuItem(MENU_HOVER + i);
                let row = row.clone();
                let color = if action == TreeAction::Delete { theme.danger } else { theme.fg_default };
                self.hoverable(div().id(ElementId::Name(format!("tree-menu-{i}").into())), key, cx)
                    .h(ITEM_H)
                    .px(px(8.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .rounded(size::RADIUS_SM)
                    .bg(self.hover_bg(key, theme.bg_hover, now))
                    .text_color(color)
                    .child(icon(action_icon(action), theme.fg_dim))
                    .child(label(action, os))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.tree.menu.as_ref().is_some_and(|(_, m)| m.closing()) {
                            return;
                        }
                        this.run_tree_action(row.clone(), action, window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let body = menu
            .frame("tree-menu", &theme, now)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_tree_menu(window, cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_tree_menu(window, cx)))
            .children(if closing { Vec::new() } else { items });
        Some(place(menu.position(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::{new_item_parent, validate_name};
    use crate::tree_model::TreeRow;
    use fude_fs::RelPath;

    fn row(rel: &str, is_dir: bool) -> TreeRow {
        let rel = RelPath::new(rel).unwrap();
        TreeRow { name: rel.as_str().rsplit('/').next().unwrap().into(), rel, depth: 0, is_dir, expanded: false, mark: None }
    }

    #[test]
    fn validate_name_rules() {
        assert!(validate_name("a.rs").is_ok());
        assert!(validate_name("作業 メモ.md").is_ok());
        for bad in ["", "  ", "a/b", "a\\b", ".", "..", "a:b", "a*b", "a?b", "a\"b", "a<b", "a|b"] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn new_item_goes_into_folder_or_beside_file() {
        assert_eq!(new_item_parent(Some(&row("src/ui", true))), RelPath::new("src/ui").unwrap());
        assert_eq!(new_item_parent(Some(&row("src/main.rs", false))), RelPath::new("src").unwrap());
        assert_eq!(new_item_parent(Some(&row("a.md", false))), RelPath::root());
        assert_eq!(new_item_parent(None), RelPath::root());
    }
}
