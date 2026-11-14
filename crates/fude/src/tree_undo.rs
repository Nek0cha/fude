//! ツリーの操作の取り消しとやり直し（計画8b）。履歴の規則は fude_core::tree_history。
//! ゴミ箱に送る前に保存していないタブがあれば止め、変更の無いタブは閉じる。

use fude_core::tree_history::TreeOp;
use fude_core::workspace::ProjectId;
use fude_fs::{RelPath, TrashTicket};
use gpui_kit::*;

use crate::dialog::error_text;
use crate::workspace_view::WorkspaceView;

/// ゴミ箱に送ると保存していない変更を失うタブ（target 自身か、その中）の最初のもの。
pub(crate) fn trash_blocked(target: &str, dirty: &[&str]) -> Option<String> {
    crate::editor_column::rename_blocked(target, dirty)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Undo,
    Redo,
}

/// 履歴の 1 件を戻す（進める）ための、ファイルの操作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Trash(RelPath),
    Restore(TrashTicket, RelPath),
    Rename { from: RelPath, to: RelPath },
}

impl Step {
    /// 操作のあとにツリーで選ぶパス（ゴミ箱に送った側は選ばない）。
    fn result_path(&self) -> Option<&RelPath> {
        match self {
            Step::Trash(_) => None,
            Step::Restore(_, path) => Some(path),
            Step::Rename { to, .. } => Some(to),
        }
    }

    /// エラーの文に出すパス。
    fn subject(&self) -> &RelPath {
        match self {
            Step::Trash(path) | Step::Restore(_, path) => path,
            Step::Rename { from, .. } => from,
        }
    }
}

/// op を direction の向きに動かす操作。パスが正しくなければ None（その op は捨てる）。
pub(crate) fn plan_step(op: &TreeOp<TrashTicket>, direction: Direction) -> Option<Step> {
    let rel = |p: &str| RelPath::new(p).ok();
    Some(match (op, direction) {
        // 新規はゴミ箱の中にあるかどうかを行き来する（向きによらない）。
        (TreeOp::Created { path, trashed: None }, _) => Step::Trash(rel(path)?),
        (TreeOp::Created { path, trashed: Some(t) }, _) => Step::Restore(t.clone(), rel(path)?),
        (TreeOp::Renamed { from, to }, Direction::Undo) => Step::Rename { from: rel(to)?, to: rel(from)? },
        (TreeOp::Renamed { from, to }, Direction::Redo) => Step::Rename { from: rel(from)?, to: rel(to)? },
        (TreeOp::Deleted { path, ticket }, Direction::Undo) => Step::Restore(ticket.clone(), rel(path)?),
        (TreeOp::Deleted { path, .. }, Direction::Redo) => Step::Trash(rel(path)?),
    })
}

/// 操作が成功したあとの op。ゴミ箱に送ったときは新しい札を持たせる。
pub(crate) fn next_op(op: TreeOp<TrashTicket>, new_ticket: Option<TrashTicket>) -> TreeOp<TrashTicket> {
    match (op, new_ticket) {
        (TreeOp::Created { path, .. }, ticket) => TreeOp::Created { path, trashed: ticket },
        (TreeOp::Deleted { path, ticket }, new) => TreeOp::Deleted { path, ticket: new.unwrap_or(ticket) },
        (op, _) => op,
    }
}

fn parent_of(rel: &RelPath) -> RelPath {
    rel.as_str().rsplit_once('/').and_then(|(p, _)| RelPath::new(p).ok()).unwrap_or_else(RelPath::root)
}

impl WorkspaceView {
    pub(crate) fn undo_tree_op(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_tree_history(Direction::Undo, window, cx);
    }

    pub(crate) fn redo_tree_op(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_tree_history(Direction::Redo, window, cx);
    }

    fn put_back(&mut self, project: ProjectId, op: TreeOp<TrashTicket>, direction: Direction) {
        let history = self.tree_history(project);
        match direction {
            Direction::Undo => history.restore_undo(op),
            Direction::Redo => history.restore_redo(op),
        }
    }

    fn run_tree_history(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.undo_running {
            return;
        }
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        let Some(fs) = self.live_fs(project) else { return };
        let history = self.tree_history(project);
        let taken = match direction {
            Direction::Undo => history.take_undo(),
            Direction::Redo => history.take_redo(),
        };
        let Some(op) = taken else { return };
        let Some(step) = plan_step(&op, direction) else { return };

        let views = self.project_views(project);
        let dirty: Vec<String> = views.iter().filter(|v| v.read(cx).is_dirty()).map(|v| v.read(cx).rel.clone()).collect();
        let dirty: Vec<&str> = dirty.iter().map(String::as_str).collect();
        let blocked = match &step {
            Step::Trash(path) => trash_blocked(path.as_str(), &dirty),
            Step::Rename { to, .. } => crate::editor_column::rename_blocked(to.as_str(), &dirty),
            Step::Restore(..) => None,
        };
        if let Some(blocked) = blocked {
            self.put_back(project, op, direction);
            let message = format!("{blocked} に保存していない変更があります。保存するか閉じてから、もう一度試してください。");
            self.show_tree_error(message, window, cx);
            return;
        }
        if let Step::Trash(path) = &step {
            self.close_clean_tabs(project, path, window, cx);
        }

        self.tree.undo_running = true;
        let task = cx.background_spawn({
            let step = step.clone();
            async move {
                match step {
                    Step::Trash(path) => fs.trash(&path).map(Some),
                    Step::Restore(ticket, path) => fs.restore(&ticket, &path).map(|()| None),
                    Step::Rename { from, to } => fs.rename(&from, &to).map(|()| None),
                }
            }
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            cx.update_window(window_handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.tree.undo_running = false;
                    this.finish_tree_history(project, op, direction, step, result, window, cx);
                })
                .ok();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_tree_history(
        &mut self,
        project: ProjectId,
        op: TreeOp<TrashTicket>,
        direction: Direction,
        step: Step,
        result: std::io::Result<Option<TrashTicket>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut reload = vec![parent_of(step.subject())];
        if let Step::Rename { to, .. } = &step {
            reload.push(parent_of(to));
        }
        for dir in reload {
            self.tree.models.entry(project).or_default().expand(&dir);
            self.load_tree_dir(project, dir, cx);
        }
        let new_ticket = match result {
            Ok(ticket) => ticket,
            Err(e) => {
                // 失敗した 1 件は捨てる（その先の履歴は残す）。
                let what = match direction {
                    Direction::Undo => "元に戻せません",
                    Direction::Redo => "やり直せません",
                };
                self.show_tree_error(error_text(what, step.subject().as_str(), &e), window, cx);
                return;
            }
        };
        if new_ticket.as_ref().is_some_and(|t| t.id.is_empty()) {
            let message = format!("{} をゴミ箱に移しましたが、もう一度元に戻すことはできません。", step.subject().as_str());
            self.show_tree_error(message, window, cx);
            return;
        }
        if let Step::Rename { from, to } = &step {
            self.editor_renamed(project, from.as_str(), to.as_str(), cx);
        }
        if let Some(path) = step.result_path() {
            self.tree.selected = Some(path.clone());
        }
        let op = next_op(op, new_ticket);
        let history = self.tree_history(project);
        match direction {
            Direction::Undo => history.done_undo(op),
            Direction::Redo => history.done_redo(op),
        }
        cx.notify();
    }

    /// path とその中を開いている、変更の無いタブを閉じる（ゴミ箱に送る前）。
    fn close_clean_tabs(&mut self, project: ProjectId, path: &RelPath, window: &mut Window, cx: &mut Context<Self>) {
        let dirty: Vec<String> =
            self.project_views(project).iter().filter(|v| v.read(cx).is_dirty()).map(|v| v.read(cx).rel.clone()).collect();
        let rels: Vec<String> = self
            .workspace
            .projects()
            .iter()
            .find(|p| p.id == project)
            .map(|p| p.editor.items().iter().map(|i| i.rel.clone()).collect())
            .unwrap_or_default();
        for rel in rels {
            let inside = crate::editor_column::rename_blocked(path.as_str(), &[rel.as_str()]).is_some();
            if !inside || dirty.contains(&rel) {
                continue;
            }
            let index = self.workspace.projects().iter().find(|p| p.id == project).and_then(|p| p.editor.position(&rel));
            if let Some(index) = index {
                self.close_editor_tab(project, index, window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use fude_core::tree_history::TreeOp;
    use fude_fs::{RelPath, TrashTicket};

    use super::{Direction, Step, next_op, plan_step, trash_blocked};

    fn rel(p: &str) -> RelPath {
        RelPath::new(p).unwrap()
    }

    fn t(id: &str) -> TrashTicket {
        TrashTicket { id: id.into() }
    }

    #[test]
    fn trash_blocked_checks_folder_children() {
        assert_eq!(trash_blocked("n.md", &["n.md"]), Some("n.md".to_string()));
        assert_eq!(trash_blocked("d", &["d/a.rs"]), Some("d/a.rs".to_string()));
        assert_eq!(trash_blocked("d", &["dd/a.rs"]), None);
        assert_eq!(trash_blocked("d", &[]), None);
    }

    #[test]
    fn steps_for_each_op() {
        let created = TreeOp::Created { path: "n.md".into(), trashed: None };
        assert_eq!(plan_step(&created, Direction::Undo), Some(Step::Trash(rel("n.md"))));
        let trashed = TreeOp::Created { path: "n.md".into(), trashed: Some(t("1")) };
        assert_eq!(plan_step(&trashed, Direction::Redo), Some(Step::Restore(t("1"), rel("n.md"))));
        let renamed = TreeOp::Renamed { from: "a.md".into(), to: "b.md".into() };
        assert_eq!(plan_step(&renamed, Direction::Undo), Some(Step::Rename { from: rel("b.md"), to: rel("a.md") }));
        assert_eq!(plan_step(&renamed, Direction::Redo), Some(Step::Rename { from: rel("a.md"), to: rel("b.md") }));
        let deleted = TreeOp::Deleted { path: "d".into(), ticket: t("2") };
        assert_eq!(plan_step(&deleted, Direction::Undo), Some(Step::Restore(t("2"), rel("d"))));
        assert_eq!(plan_step(&deleted, Direction::Redo), Some(Step::Trash(rel("d"))));
    }

    #[test]
    fn next_op_takes_new_ticket() {
        let created = TreeOp::Created { path: "n.md".into(), trashed: None };
        assert_eq!(next_op(created.clone(), Some(t("1"))), TreeOp::Created { path: "n.md".into(), trashed: Some(t("1")) });
        let trashed = TreeOp::Created { path: "n.md".into(), trashed: Some(t("1")) };
        assert_eq!(next_op(trashed, None), created);
        let deleted = TreeOp::Deleted { path: "d".into(), ticket: t("2") };
        assert_eq!(next_op(deleted.clone(), Some(t("3"))), TreeOp::Deleted { path: "d".into(), ticket: t("3") });
        assert_eq!(next_op(deleted.clone(), None), deleted);
    }
}
