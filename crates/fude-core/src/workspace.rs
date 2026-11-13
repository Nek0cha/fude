//! プロジェクト、ターミナルタブ、分割の状態を管理する。GUI には依存しない。

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProjectId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaneId(u64);

/// ターミナルで動かすシェルの種類。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellKind {
    Native,
    Wsl { distro: String },
}

impl ShellKind {
    /// タブやペインに表示する短い名前。
    pub fn label(&self) -> String {
        match self {
            ShellKind::Native if cfg!(target_os = "macos") => "zsh".into(),
            ShellKind::Native => "pwsh".into(),
            ShellKind::Wsl { distro } => format!("wsl·{distro}"),
        }
    }
}

pub struct Pane {
    pub id: PaneId,
    pub kind: ShellKind,
}

/// ターミナルのタブ。ペインを 1 つか 2 つ（左右の分割）持つ。
pub struct Tab {
    pub id: TabId,
    pub panes: Vec<Pane>,
    pub focused: usize,
}

impl Tab {
    /// フォーカス中のペインの名前。
    pub fn title(&self) -> String {
        self.panes[self.focused].kind.label()
    }
}

pub struct Project {
    pub id: ProjectId,
    pub root: PathBuf,
    pub name: String,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<TabId>,
}

impl Project {
    pub fn active_tab(&self) -> Option<&Tab> {
        let id = self.active_tab?;
        self.tabs.iter().find(|t| t.id == id)
    }

    fn active_index(&self) -> Option<usize> {
        let id = self.active_tab?;
        self.tabs.iter().position(|t| t.id == id)
    }
}

#[derive(Default)]
pub struct Workspace {
    projects: Vec<Project>,
    active: Option<ProjectId>,
    next_id: u64,
}

impl Workspace {
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn active_project(&self) -> Option<&Project> {
        let id = self.active?;
        self.projects.iter().find(|p| p.id == id)
    }

    fn active_project_mut(&mut self) -> Option<&mut Project> {
        let id = self.active?;
        self.projects.iter_mut().find(|p| p.id == id)
    }

    fn alloc(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn make_tab(&mut self, kind: ShellKind) -> Tab {
        let id = TabId(self.alloc());
        let pane = Pane { id: PaneId(self.alloc()), kind };
        Tab { id, panes: vec![pane], focused: 0 }
    }

    /// root は呼び出し側で正規化済みとする。同じ root があればそれを選び、その id を返す。
    /// 新しいプロジェクトには既定のシェルのタブを 1 つ作る。
    pub fn open_project(&mut self, root: PathBuf, default_shell: ShellKind) -> ProjectId {
        if let Some(p) = self.projects.iter().find(|p| p.root == root) {
            let id = p.id;
            self.active = Some(id);
            return id;
        }
        let id = ProjectId(self.alloc());
        let tab = self.make_tab(default_shell);
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        self.projects.push(Project { id, root, name, active_tab: Some(tab.id), tabs: vec![tab] });
        self.active = Some(id);
        id
    }

    pub fn activate_project(&mut self, id: ProjectId) {
        if self.projects.iter().any(|p| p.id == id) {
            self.active = Some(id);
        }
    }

    /// ⌘1〜9 用。index は 0 始まり。範囲外なら何もしない。
    pub fn activate_project_at(&mut self, index: usize) {
        if let Some(p) = self.projects.get(index) {
            self.active = Some(p.id);
        }
    }

    /// 選択中のプロジェクトにタブを足して選ぶ。プロジェクトが無ければ None。
    pub fn new_tab(&mut self, kind: ShellKind) -> Option<TabId> {
        self.active?;
        let tab = self.make_tab(kind);
        let id = tab.id;
        let project = self.active_project_mut()?;
        project.tabs.push(tab);
        project.active_tab = Some(id);
        Some(id)
    }

    pub fn activate_tab(&mut self, id: TabId) {
        if let Some(p) = self.active_project_mut()
            && p.tabs.iter().any(|t| t.id == id)
        {
            p.active_tab = Some(id);
        }
    }

    /// delta は +1 か -1。端では反対側へ回り込む。
    pub fn cycle_tab(&mut self, delta: i32) {
        let Some(p) = self.active_project_mut() else { return };
        let Some(i) = p.active_index() else { return };
        let len = p.tabs.len() as i32;
        let next = (i as i32 + delta).rem_euclid(len) as usize;
        p.active_tab = Some(p.tabs[next].id);
    }

    /// 閉じたタブが選択中なら、右隣、無ければ左隣を選ぶ。
    pub fn close_tab(&mut self, id: TabId) {
        let Some(p) = self.active_project_mut() else { return };
        let Some(i) = p.tabs.iter().position(|t| t.id == id) else { return };
        p.tabs.remove(i);
        if p.active_tab == Some(id) {
            let next = if i < p.tabs.len() { Some(i) } else { i.checked_sub(1) };
            p.active_tab = next.map(|n| p.tabs[n].id);
        }
    }

    /// 選択中のタブを左右に分割する。新しいペインはフォーカス中のペインと同じ種類で、
    /// フォーカスを移す。すでに 2 つなら何もしない。
    pub fn split_active_tab(&mut self) {
        let Some(kind) = self
            .active_project()
            .and_then(Project::active_tab)
            .filter(|tab| tab.panes.len() < 2)
            .map(|tab| tab.panes[tab.focused].kind.clone())
        else {
            return;
        };
        let pane = Pane { id: PaneId(self.alloc()), kind };
        if let Some(p) = self.active_project_mut()
            && let Some(i) = p.active_index()
        {
            let tab = &mut p.tabs[i];
            tab.panes.push(pane);
            tab.focused = tab.panes.len() - 1;
        }
    }
}

/// プロジェクトのアイコンの色を、root から決める（6 色の中から）。
/// FNV-1a を使い、実行のたびに同じ値になる。
pub fn project_color_index(root: &Path) -> usize {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (hash % 6) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ws_with(roots: &[&str]) -> Workspace {
        let mut ws = Workspace::default();
        for root in roots {
            ws.open_project(PathBuf::from(root), ShellKind::Native);
        }
        ws
    }

    fn active(ws: &Workspace) -> &Project {
        ws.active_project().expect("選択中のプロジェクトがある")
    }

    fn tab_ids(ws: &Workspace) -> Vec<TabId> {
        active(ws).tabs.iter().map(|t| t.id).collect()
    }

    #[test]
    fn open_project_creates_one_native_tab() {
        let ws = ws_with(&["/w/fude"]);
        let p = active(&ws);
        assert_eq!(p.name, "fude");
        assert_eq!(p.tabs.len(), 1);
        assert_eq!(p.active_tab, Some(p.tabs[0].id));
        assert_eq!(p.tabs[0].panes[0].kind, ShellKind::Native);
    }

    #[test]
    fn open_same_root_activates_existing() {
        let mut ws = Workspace::default();
        let a = ws.open_project(PathBuf::from("/w/a"), ShellKind::Native);
        ws.open_project(PathBuf::from("/w/b"), ShellKind::Native);
        let again = ws.open_project(PathBuf::from("/w/a"), ShellKind::Native);
        assert_eq!(ws.projects().len(), 2);
        assert_eq!(again, a);
        assert_eq!(active(&ws).id, a);
    }

    #[test]
    fn activate_project_at_out_of_range_is_noop() {
        let mut ws = ws_with(&["/w/a", "/w/b"]);
        let before = active(&ws).id;
        ws.activate_project_at(5);
        assert_eq!(active(&ws).id, before);
        ws.activate_project_at(0);
        assert_eq!(active(&ws).id, ws.projects()[0].id);
    }

    #[test]
    fn new_tab_appends_and_activates() {
        let mut ws = ws_with(&["/w/a"]);
        let id = ws.new_tab(ShellKind::Native).unwrap();
        assert_eq!(active(&ws).tabs.len(), 2);
        assert_eq!(active(&ws).active_tab, Some(id));
    }

    #[test]
    fn cycle_tab_wraps() {
        let mut ws = ws_with(&["/w/a"]);
        ws.new_tab(ShellKind::Native);
        ws.new_tab(ShellKind::Native);
        let ids = tab_ids(&ws);
        assert_eq!(active(&ws).active_tab, Some(ids[2]));
        ws.cycle_tab(1);
        assert_eq!(active(&ws).active_tab, Some(ids[0]));
        ws.cycle_tab(-1);
        assert_eq!(active(&ws).active_tab, Some(ids[2]));
    }

    #[test]
    fn close_active_tab_selects_right_then_left() {
        let mut ws = ws_with(&["/w/a"]);
        ws.new_tab(ShellKind::Native);
        ws.new_tab(ShellKind::Native);
        let ids = tab_ids(&ws);
        ws.activate_tab(ids[1]);
        ws.close_tab(ids[1]);
        assert_eq!(active(&ws).active_tab, Some(ids[2]));
        ws.close_tab(ids[2]);
        assert_eq!(active(&ws).active_tab, Some(ids[0]));
    }

    #[test]
    fn close_last_tab_leaves_no_active() {
        let mut ws = ws_with(&["/w/a"]);
        let only = tab_ids(&ws)[0];
        ws.close_tab(only);
        assert!(active(&ws).tabs.is_empty());
        assert_eq!(active(&ws).active_tab, None);
        assert!(ws.new_tab(ShellKind::Native).is_some());
    }

    #[test]
    fn split_adds_second_pane_once() {
        let mut ws = ws_with(&["/w/a"]);
        ws.split_active_tab();
        ws.split_active_tab();
        let tab = active(&ws).active_tab().unwrap();
        assert_eq!(tab.panes.len(), 2);
        assert_eq!(tab.focused, 1);
    }

    #[test]
    fn wsl_label() {
        let kind = ShellKind::Wsl { distro: "Ubuntu".into() };
        assert_eq!(kind.label(), "wsl·Ubuntu");
    }

    #[test]
    fn project_color_index_is_stable() {
        let root = Path::new("/w/a");
        assert_eq!(project_color_index(root), project_color_index(root));
        let values: std::collections::HashSet<usize> = ('a'..='z')
            .map(|c| project_color_index(Path::new(&format!("/w/{c}"))))
            .collect();
        assert!(values.iter().all(|&v| v < 6));
        assert!(values.len() >= 2);
    }
}
