//! ツリーの右クリックメニュー（設計書 §4.3。「Claude に渡す」は計画7）。
//! 新規と名前の変更は、ツリーのその行の場所（新規は作り先のフォルダの中の先頭）に入力欄を出し、Enter か
//! フォーカスが外れたときに確定、Esc で取り消す。削除はゴミ箱に送る（計画8b）。

use std::path::Path;
use std::time::Instant;

use fude_core::claude::mention;
use fude_core::keymap::Platform;
use fude_core::tree_history::{TreeHistory, TreeOp, name_stem_len};
use fude_core::workspace::ProjectId;
use fude_fs::{RelPath, TrashTicket};
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::dialog::{ButtonRole, Dialog, DialogButton, error_text};
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
    SendToClaude,
    Reveal,
}

/// 設計書 §4.3 の並び。
const ACTIONS: [TreeAction; 7] = [
    TreeAction::NewFile,
    TreeAction::NewFolder,
    TreeAction::Rename,
    TreeAction::Delete,
    TreeAction::CopyPath,
    TreeAction::SendToClaude,
    TreeAction::Reveal,
];

/// 開いているツリーのメニュー。row が None なら何も無いところのメニュー。index はキーで選んでいる項目
/// （マウスで出したときは None）。
pub(crate) struct TreeMenuState {
    pub row: Option<TreeRow>,
    pub popup: Popup,
    pub index: Option<usize>,
}

/// メニューに並べる項目。行が無いところでは、新規とエクスプローラー（Finder）で表示だけ。
fn menu_actions(has_row: bool) -> Vec<TreeAction> {
    ACTIONS
        .iter()
        .copied()
        .filter(|a| has_row || matches!(a, TreeAction::NewFile | TreeAction::NewFolder | TreeAction::Reveal))
        .collect()
}

/// ↑↓ で動かした先の項目。選択が無ければ ↓ は先頭、↑ は末尾。端で止まる。
fn step_menu(index: Option<usize>, len: usize, down: bool) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(match (index, down) {
        (None, true) => 0,
        (None, false) => last,
        (Some(i), true) => (i + 1).min(last),
        (Some(i), false) => i.saturating_sub(1),
    })
}

/// キーで出すメニューの左上。行の左端から 24px 右、行の下端。scroll_y は一覧のスクロールのずれ（下へ送ると負）。
/// 見えていない行は Nearest で一覧の端まで送られるので、一覧の中に収める。
fn key_menu_position(list: Bounds<Pixels>, row_index: usize, scroll_y: Pixels) -> Point<Pixels> {
    let row_h = crate::tree_view::ROW_H;
    let y = list.origin.y + row_h * (row_index as f32 + 1.) + scroll_y;
    let y = y.max(list.origin.y + row_h).min(list.origin.y + list.size.height);
    point(list.origin.x + px(24.), y)
}

/// メニューの項目の光り方。キーで選んでいる間は選んだ項目だけを光らせ、ホバーとの二重の光りを避ける。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemLight {
    Hover,
    Selected,
    Off,
}

fn item_light(index: Option<usize>, item: usize) -> ItemLight {
    match index {
        None => ItemLight::Hover,
        Some(i) if i == item => ItemLight::Selected,
        Some(_) => ItemLight::Off,
    }
}

/// 新規か名前の変更が済んだあとの選択。新規は作ったものを選ぶ。名前の変更は、選んでいた行（とフォルダの中の行）
/// を新しいパスに付け替え、それ以外の選択はそのまま。
fn selection_after_edit(kind: EditKind, selected: Option<&RelPath>, target: &RelPath, dest: &RelPath) -> Option<RelPath> {
    if kind != EditKind::Rename {
        return Some(dest.clone());
    }
    let selected = selected?;
    if selected == target {
        return Some(dest.clone());
    }
    match selected.as_str().strip_prefix(target.as_str()).and_then(|rest| rest.strip_prefix('/')) {
        Some(rest) => dest.join(rest).ok().or_else(|| Some(selected.clone())),
        None => Some(selected.clone()),
    }
}

fn label(action: TreeAction, os: Platform) -> &'static str {
    match (action, os) {
        (TreeAction::NewFile, _) => "新規ファイル",
        (TreeAction::NewFolder, _) => "新規フォルダ",
        (TreeAction::Rename, _) => "名前の変更",
        (TreeAction::Delete, _) => "削除",
        (TreeAction::CopyPath, _) => "パスをコピー",
        (TreeAction::SendToClaude, _) => "Claude に渡す",
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
        TreeAction::SendToClaude => IconName::Send,
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

/// ツリーの行を Claude に渡す文字列。フォルダは末尾に `/` を付ける。
fn tree_mention(row: &TreeRow) -> String {
    let rel = row.rel.as_str();
    if row.is_dir { mention(&format!("{rel}/"), None) } else { mention(rel, None) }
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
    /// 入れる名前がフォルダのものか（アイコンと最初の選択に使う）。
    pub is_dir: bool,
    pub input: Entity<InputState>,
    _sub: Subscription,
}

/// メニューの HoverKey の番号。
const MENU_HOVER: usize = 400;

impl WorkspaceView {
    /// プロジェクトのツリーの操作の履歴。
    pub(crate) fn tree_history(&mut self, project: ProjectId) -> &mut TreeHistory<TrashTicket> {
        self.tree.history.entry(project).or_default()
    }

    /// ツリーの操作の失敗を知らせ、閉じたらツリーへフォーカスを返す。
    pub(crate) fn show_tree_error(&mut self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.tree.focus.clone();
        self.show_error_to(message, Some(focus), window, cx);
    }

    pub(crate) fn open_tree_menu(&mut self, row: Option<TreeRow>, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.tree.menu = Some(TreeMenuState { row, popup: Popup::open(position, window, cx), index: None });
        cx.notify();
    }

    /// Shift+F10 とメニューキー。選んでいる行の下に出し、先頭の項目を選んでおく。選択が並びに無ければ、
    /// 何も無いところのメニューを一覧の左上に出す。
    pub(crate) fn open_tree_menu_by_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list = self.tree.list_bounds;
        let selected = crate::tree_nav::selected_row(&self.tree.rows, self.tree.selected.as_ref()).map(|(ix, row)| (ix, row.clone()));
        let (row, position) = match selected {
            Some((ix, row)) => {
                self.tree.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                let scroll_y = self.tree.scroll.0.borrow().base_handle.offset().y;
                (Some(row), key_menu_position(list, ix, scroll_y))
            }
            None => (None, list.origin + point(px(24.), px(24.))),
        };
        self.tree.menu = Some(TreeMenuState { row, popup: Popup::open(position, window, cx), index: Some(0) });
        cx.notify();
    }

    /// キーでメニューを操作する。Esc で閉じ、↑↓ で選び、Enter で選んだ項目を行う。
    fn tree_menu_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.tree.menu.as_mut() else { return };
        if menu.popup.closing() {
            return;
        }
        let actions = menu_actions(menu.row.is_some());
        match key {
            "escape" => self.close_tree_menu(window, cx),
            "up" | "down" => {
                menu.index = step_menu(menu.index, actions.len(), key == "down");
                cx.notify();
            }
            "enter" => {
                if let Some(action) = menu.index.and_then(|i| actions.get(i).copied()) {
                    let row = menu.row.clone();
                    self.run_tree_action(row, action, window, cx);
                }
            }
            _ => {}
        }
    }

    fn close_tree_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.menu.as_mut().is_some_and(|m| m.popup.close(Instant::now())) {
            window.focus(&self.tree.focus, cx);
            cx.notify();
        }
    }

    pub(crate) fn run_tree_action(&mut self, row: Option<TreeRow>, action: TreeAction, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tree_menu(window, cx);
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        match action {
            TreeAction::NewFile => {
                self.start_tree_edit(project, EditKind::NewFile, new_item_parent(row.as_ref()), "", false, window, cx)
            }
            TreeAction::NewFolder => {
                self.start_tree_edit(project, EditKind::NewFolder, new_item_parent(row.as_ref()), "", true, window, cx)
            }
            TreeAction::Rename => {
                if let Some(row) = row {
                    self.start_tree_edit(project, EditKind::Rename, row.rel.clone(), &row.name, row.is_dir, window, cx);
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
            TreeAction::SendToClaude => {
                if let Some(row) = row {
                    self.send_to_claude(project, tree_mention(&row), cx);
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

    #[allow(clippy::too_many_arguments)]
    fn start_tree_edit(
        &mut self,
        project: ProjectId,
        kind: EditKind,
        target: RelPath,
        initial: &str,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if kind != EditKind::Rename {
            // 絞り込み中と「変更だけ」の表示ではフォルダを開けないので、ふつうのツリーに戻す。
            let model = self.tree.models.entry(project).or_default();
            if model.filtering() || model.changed_only() {
                model.set_changed_only(false);
                model.set_filter("", None);
                if let Some(filter) = self.tree.filter.clone() {
                    filter.update(cx, |s, cx| s.set_value("", window, cx));
                }
            }
            // 作り先の祖先もすべて開く（絞り込みで見えていたフォルダは、ふつうの表示では閉じていることがある）。
            for dir in self.tree.models.entry(project).or_default().reveal(&target) {
                self.load_tree_dir(project, dir, cx);
            }
        }
        let initial = initial.to_string();
        let stem = name_stem_len(&initial, is_dir);
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(initial, window, cx);
            state.set_selected_range(0..stem, cx);
            state
        });
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } => this.commit_tree_edit(window, cx),
            // 欄の外をクリックしたときなど。エラーのダイアログに移ったときは確定しない。
            InputEvent::Blur if !this.dialog_open() => this.commit_tree_edit(window, cx),
            _ => {}
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.tree.edit = Some(TreeEdit { project, kind, target, is_dir, input, _sub: sub });
        self.tree.scroll_to_edit = true;
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
        let unchanged = edit.kind == EditKind::Rename && edit.target.as_str().rsplit('/').next() == Some(name.as_str());
        if name.is_empty() || unchanged {
            self.cancel_tree_edit(window, cx);
            return;
        }
        let input_focus = edit.input.read(cx).focus_handle(cx);
        if let Err(why) = validate_name(&name) {
            self.show_error_to(format!("{why}: {name}"), Some(input_focus), window, cx);
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
        if kind == EditKind::Rename {
            let views = self.project_views(project);
            let dirty: Vec<String> = views.iter().filter(|v| v.read(cx).is_dirty()).map(|v| v.read(cx).rel.clone()).collect();
            let dirty: Vec<&str> = dirty.iter().map(String::as_str).collect();
            if let Some(blocked) = crate::editor_column::rename_blocked(dest.as_str(), &dirty) {
                let message = format!("{blocked} に保存していない変更があります。保存するか閉じてから、名前を変えてください。");
                self.show_error_to(message, Some(input_focus), window, cx);
                return;
            }
        }
        self.tree.edit = None;
        window.focus(&self.tree.focus, cx);
        // 終わるまで取り消しを受け付けない（終わる前の Ctrl+Z で、前の操作を取り消さないため）。
        self.tree_history(project).begin_op();
        let task = cx.background_spawn({
            let dest = dest.clone();
            let target = target.clone();
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
                    this.tree_history(project).end_op();
                    if let Err(e) = result {
                        let what = match kind {
                            EditKind::NewFile => "ファイルを作れません",
                            EditKind::NewFolder => "フォルダを作れません",
                            EditKind::Rename => "名前を変えられません",
                        };
                        this.show_tree_error(error_text(what, dest.as_str(), &e), window, cx);
                        return;
                    }
                    let op = match kind {
                        EditKind::Rename => {
                            this.editor_renamed(project, target.as_str(), dest.as_str(), cx);
                            TreeOp::Renamed { from: target.as_str().to_string(), to: dest.as_str().to_string() }
                        }
                        _ => TreeOp::Created { path: dest.as_str().to_string(), trashed: None },
                    };
                    this.tree_history(project).push(op);
                    // 選択を新しいパスへ移す（古いパスは並びから消え、キーの操作が効かなくなるため）。
                    let selected = selection_after_edit(kind, this.tree.selected.as_ref(), &target, &dest);
                    this.select_tree_row(selected, cx);
                    // 作り先のフォルダを開いて読み直す（監視の通知を待たない）。
                    this.tree.models.entry(project).or_default().expand(&parent);
                    this.load_tree_dir(project, parent.clone(), cx);
                    if kind == EditKind::NewFile {
                        this.open_in_editor(project, dest.as_str().to_string(), None, None, window, cx);
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
            message: format!("{} をゴミ箱に移しますか？", row.name),
            buttons: vec![
                DialogButton { label: "キャンセル", role: ButtonRole::Cancel },
                DialogButton { label: "ゴミ箱に移す", role: ButtonRole::Danger },
            ],
            refocus: Some(self.tree.focus.clone()),
            on_choose: Box::new(move |i, this, window, cx| {
                if i != 1 {
                    return;
                }
                let window_handle = window.window_handle();
                let Some(fs) = this.live_fs(project) else { return };
                let rel = row.rel.clone();
                // 削除の後の選択を先に決めておく（監視の読み直しが先に済むと、消した行が並びから消えるため）。
                let planned = crate::tree_nav::after_removal(&this.tree.rows, &row.rel);
                this.tree_history(project).begin_op();
                let task = cx.background_spawn(async move { fs.trash(&rel) });
                let parent =
                    row.rel.as_str().rsplit_once('/').and_then(|(p, _)| RelPath::new(p).ok()).unwrap_or_else(RelPath::root);
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    cx.update_window(window_handle, |_, window, cx| {
                        this.update(cx, |this, cx| {
                            this.tree_history(project).end_op();
                            match result {
                                Ok(ticket) => {
                                    // 消した行を選んでいたら、次の行（無ければ前の行）へ移す。
                                    if this.tree.selected.as_ref() == Some(&row.rel) {
                                        let next = crate::tree_nav::next_after_delete(&this.tree.rows, &row.rel, planned);
                                        this.select_tree_row(next, cx);
                                    }
                                    this.load_tree_dir(project, parent, cx);
                                    if ticket.id.is_empty() {
                                        let message = format!("{} をゴミ箱に移しましたが、取り消しには使えません。", row.name);
                                        this.show_tree_error(message, window, cx);
                                    } else {
                                        let op = TreeOp::Deleted { path: row.rel.as_str().to_string(), ticket };
                                        this.tree_history(project).push(op);
                                    }
                                }
                                Err(e) => this.show_tree_error(error_text("ゴミ箱に移せません", row.rel.as_str(), &e), window, cx),
                            }
                        })
                        .ok();
                    })
                    .ok();
                })
                .detach();
            }),
        };
        self.show_dialog(dialog, window, cx);
    }

    pub(crate) fn render_tree_menu(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Option<Deferred> {
        let state = self.tree.menu.as_ref()?;
        let (row, menu, index) = (state.row.clone(), &state.popup, state.index);
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
        let items: Vec<AnyElement> = menu_actions(row.is_some())
            .into_iter()
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
                    // キーで選んでいる項目は、ホバーと同じ色にする。キーで選んでいる間はほかの項目を光らせない。
                    .bg(match item_light(index, i) {
                        ItemLight::Hover => self.hover_bg(key, theme.bg_hover, now),
                        ItemLight::Selected => theme.bg_hover,
                        ItemLight::Off => Rgba { r: 0., g: 0., b: 0., a: 0. },
                    })
                    // キーで選んでいる間にマウスを動かしたら、その項目を選ぶ（光りを 1 つに保つ）。
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        if let Some(menu) = this.tree.menu.as_mut()
                            && menu.index.is_some_and(|ix| ix != i)
                        {
                            menu.index = Some(i);
                            cx.notify();
                        }
                    }))
                    .text_color(color)
                    .child(icon(action_icon(action), theme.fg_dim))
                    .child(label(action, os))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.tree.menu.as_ref().is_some_and(|m| m.popup.closing()) {
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
                let key = ev.keystroke.key.clone();
                this.tree_menu_key(&key, window, cx);
            }))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_tree_menu(window, cx)))
            .children(if closing { Vec::new() } else { items });
        Some(place(menu.position(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::{EditKind, ItemLight, TreeAction, item_light, key_menu_position, menu_actions, new_item_parent, selection_after_edit, step_menu, tree_mention, validate_name};
    use gpui_kit::{Bounds, Pixels, point, px, size};
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

    #[test]
    fn tree_mention_marks_folders() {
        assert_eq!(tree_mention(&row("src/main.rs", false)), "@src/main.rs ");
        assert_eq!(tree_mention(&row("src/ui", true)), "@src/ui/ ");
    }

    #[test]
    fn menu_actions_without_row_are_three() {
        assert_eq!(menu_actions(false), vec![TreeAction::NewFile, TreeAction::NewFolder, TreeAction::Reveal]);
        assert_eq!(menu_actions(true).len(), 7);
    }

    #[test]
    fn selection_follows_rename_and_creation() {
        let r = |p: &str| RelPath::new(p).unwrap();
        // 名前を変えた行を選んでいれば、新しい名前を選ぶ。フォルダの中の行は付け替える。
        assert_eq!(selection_after_edit(EditKind::Rename, Some(&r("a.txt")), &r("a.txt"), &r("b.txt")), Some(r("b.txt")));
        assert_eq!(selection_after_edit(EditKind::Rename, Some(&r("d/x")), &r("d"), &r("e")), Some(r("e/x")));
        assert_eq!(selection_after_edit(EditKind::Rename, Some(&r("dd")), &r("d"), &r("e")), Some(r("dd")));
        assert_eq!(selection_after_edit(EditKind::Rename, None, &r("d"), &r("e")), None);
        // 新規は作ったものを選ぶ。
        assert_eq!(selection_after_edit(EditKind::NewFolder, Some(&r("a.txt")), &r("src"), &r("src/new")), Some(r("src/new")));
        assert_eq!(selection_after_edit(EditKind::NewFile, None, &r("src"), &r("src/n.rs")), Some(r("src/n.rs")));
    }

    #[test]
    fn only_one_menu_item_lights() {
        // マウスで出したメニューはホバーだけで光る。
        assert_eq!(item_light(None, 2), ItemLight::Hover);
        // キーで選んでいる間は、選んだ項目だけが光り、ほかはホバーしても光らない。
        assert_eq!(item_light(Some(2), 2), ItemLight::Selected);
        assert_eq!(item_light(Some(2), 3), ItemLight::Off);
    }

    #[test]
    fn step_menu_stops_at_ends() {
        assert_eq!(step_menu(None, 7, true), Some(0));
        assert_eq!(step_menu(None, 7, false), Some(6));
        assert_eq!(step_menu(Some(6), 7, true), Some(6));
        assert_eq!(step_menu(Some(0), 7, false), Some(0));
        assert_eq!(step_menu(Some(2), 7, true), Some(3));
        assert_eq!(step_menu(None, 0, true), None);
    }

    fn list() -> Bounds<Pixels> {
        Bounds { origin: point(px(100.), px(50.)), size: size(px(200.), px(240.)) }
    }

    #[test]
    fn key_menu_position_below_row() {
        // 3 行目（0 始まりで 2）、1 行分下へ送ってある。
        assert_eq!(key_menu_position(list(), 2, px(-24.)), point(px(124.), px(98.)));
    }

    #[test]
    fn key_menu_position_stays_in_list() {
        // 見えていない行は Nearest で端まで送られるので、その端に出す。
        assert_eq!(key_menu_position(list(), 0, px(-240.)), point(px(124.), px(74.)));
        assert_eq!(key_menu_position(list(), 50, px(0.)), point(px(124.), px(290.)));
    }
}
