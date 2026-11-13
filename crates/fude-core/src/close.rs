//! 閉じるキーで何を閉じるか（設計書 §5.12）。

use crate::workspace::{PaneId, ProjectId, TabId, Workspace};

/// フォーカスのある場所。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusArea {
    /// 右パネル（ターミナルが 0 枚のときはメイン領域のエディタ）。
    RightPanel,
    /// 左パネル（ツリー）。
    LeftPanel,
    Terminal(PaneId),
    Home,
    Other,
}

/// 閉じるキーで閉じるもの。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseTarget {
    RightPanel,
    LeftPanel,
    Pane(PaneId),
    Tab(TabId),
    Project(ProjectId),
    Nothing,
}

/// フォーカスのある場所から、閉じるものを決める。フォーカスのある場所が閉じられないとき（Other、または
/// 閉じたパネル）は、選択中のタブ、右パネル、プロジェクトの順に閉じる。
pub fn close_target(ws: &Workspace, focus: FocusArea, right_panel_open: bool, left_panel_open: bool) -> CloseTarget {
    if focus == FocusArea::Home {
        return CloseTarget::Nothing;
    }
    let Some(project) = ws.active_project() else { return CloseTarget::Nothing };
    match focus {
        FocusArea::RightPanel if right_panel_open => return CloseTarget::RightPanel,
        FocusArea::LeftPanel if left_panel_open => return CloseTarget::LeftPanel,
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
    if right_panel_open {
        return CloseTarget::RightPanel;
    }
    CloseTarget::Project(project.id)
}

#[cfg(test)]
mod tests {
    use super::*;
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

        assert_eq!(close_target(&ws, FocusArea::RightPanel, true, false), CloseTarget::RightPanel);
        assert_eq!(close_target(&ws, FocusArea::LeftPanel, false, true), CloseTarget::LeftPanel);
        assert_eq!(close_target(&ws, FocusArea::Terminal(pane), false, false), CloseTarget::Tab(tab));
        assert_eq!(close_target(&ws, FocusArea::Other, true, false), CloseTarget::Tab(tab));
        assert_eq!(close_target(&ws, FocusArea::Home, false, false), CloseTarget::Nothing);

        ws.split_active_tab();
        let second = ws.active_project().unwrap().tabs[0].panes[1].id;
        assert_eq!(close_target(&ws, FocusArea::Terminal(second), false, false), CloseTarget::Pane(second));

        ws.close_tab(tab);
        assert_eq!(close_target(&ws, FocusArea::Other, true, false), CloseTarget::RightPanel);
        assert_eq!(close_target(&ws, FocusArea::Other, false, false), CloseTarget::Project(pid));
        assert_eq!(close_target(&ws, FocusArea::LeftPanel, false, false), CloseTarget::Project(pid));

        let mut empty = Workspace::default();
        assert_eq!(close_target(&empty, FocusArea::Other, false, false), CloseTarget::Nothing);
        empty.open_project_with(local("/w/b"), None, OpenMode::EditorOnly);
        assert_eq!(close_target(&empty, FocusArea::RightPanel, true, false), CloseTarget::RightPanel);
    }
}
