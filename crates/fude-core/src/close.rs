//! 閉じるキーで何を閉じるか（設計書 §5.12）。

use crate::workspace::{PaneId, ProjectId, TabId, Workspace};

/// フォーカスのある場所。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusArea {
    /// エディタ列（ターミナルが 0 枚のときはメイン領域）。
    Editor,
    /// ファイルツリー。
    Tree,
    Terminal(PaneId),
    Home,
    Other,
}

/// 閉じるキーで閉じるもの。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseTarget {
    /// エディタ列のタブ（位置）。
    EditorTab(usize),
    Tree,
    Pane(PaneId),
    Tab(TabId),
    Project(ProjectId),
    Nothing,
}

/// フォーカスのある場所から、閉じるものを決める。フォーカスのある場所が閉じられないとき（Other、
/// 閉じたツリー、空のエディタ列）は、選択中のターミナルのタブ、エディタ列の選択中のタブ、プロジェクトの順に閉じる。
pub fn close_target(ws: &Workspace, focus: FocusArea, tree_open: bool) -> CloseTarget {
    if focus == FocusArea::Home {
        return CloseTarget::Nothing;
    }
    let Some(project) = ws.active_project() else { return CloseTarget::Nothing };
    let editor_tab = project.editor.active_index().map(CloseTarget::EditorTab);
    match focus {
        FocusArea::Editor if editor_tab.is_some() => return editor_tab.unwrap_or(CloseTarget::Nothing),
        FocusArea::Tree if tree_open => return CloseTarget::Tree,
        FocusArea::Terminal(pane) => {
            if let Some(tab) = project.tabs.iter().find(|t| t.panes.iter().any(|p| p.id == pane)) {
                return if tab.panes.len() > 1 { CloseTarget::Pane(pane) } else { CloseTarget::Tab(tab.id) };
            }
        }
        _ => {}
    }
    if let Some(tab) = project.active_tab() {
        return CloseTarget::Tab(tab.id);
    }
    editor_tab.unwrap_or(CloseTarget::Project(project.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_tabs::EditorItem;
    use crate::panel::FileMode;
    use crate::workspace::{OpenMode, ProjectRoot, Workspace};
    use std::path::PathBuf;

    fn local(p: &str) -> ProjectRoot {
        ProjectRoot::Local(PathBuf::from(p))
    }

    #[test]
    fn close_target_chain() {
        let mut ws = Workspace::default();
        ws.open_project(local("/w/a"));
        let p = ws.active_project().unwrap();
        let (pid, tab, pane) = (p.id, p.tabs[0].id, p.tabs[0].panes[0].id);

        // エディタ列にフォーカスがあれば、選択中のタブ。
        ws.editor_mut(pid).unwrap().open(EditorItem { rel: "a".into(), mode: FileMode::Full });
        ws.editor_mut(pid).unwrap().open(EditorItem { rel: "b".into(), mode: FileMode::Full });
        assert_eq!(close_target(&ws, FocusArea::Editor, false), CloseTarget::EditorTab(1));
        assert_eq!(close_target(&ws, FocusArea::Tree, true), CloseTarget::Tree);
        assert_eq!(close_target(&ws, FocusArea::Terminal(pane), false), CloseTarget::Tab(tab));
        assert_eq!(close_target(&ws, FocusArea::Other, false), CloseTarget::Tab(tab));
        assert_eq!(close_target(&ws, FocusArea::Home, false), CloseTarget::Nothing);

        ws.split_active_tab();
        let second = ws.active_project().unwrap().tabs[0].panes[1].id;
        assert_eq!(close_target(&ws, FocusArea::Terminal(second), false), CloseTarget::Pane(second));

        // ターミナルのタブを閉じて 0 枚にしたら、フォーカスが Other のときはエディタのタブ、
        // エディタのタブも全部閉じたらプロジェクト。
        ws.close_tab(tab);
        assert_eq!(close_target(&ws, FocusArea::Other, false), CloseTarget::EditorTab(1));
        ws.editor_mut(pid).unwrap().close(1);
        ws.editor_mut(pid).unwrap().close(0);
        assert_eq!(close_target(&ws, FocusArea::Other, false), CloseTarget::Project(pid));
        // 閉じたツリーや空のエディタ列にフォーカスが残っていても、先へ進む。
        assert_eq!(close_target(&ws, FocusArea::Tree, false), CloseTarget::Project(pid));
        assert_eq!(close_target(&ws, FocusArea::Editor, false), CloseTarget::Project(pid));

        let mut empty = Workspace::default();
        assert_eq!(close_target(&empty, FocusArea::Other, false), CloseTarget::Nothing);
        let b = empty.open_project_with(local("/w/b"), None, OpenMode::EditorOnly);
        empty.editor_mut(b).unwrap().open(EditorItem { rel: "x".into(), mode: FileMode::Full });
        assert_eq!(close_target(&empty, FocusArea::Editor, false), CloseTarget::EditorTab(0));
    }
}
