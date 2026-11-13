//! プロジェクト、ターミナルタブ、分割の状態を管理する。GUI には依存しない。

use std::path::Path;

use serde::{Deserialize, Serialize};

pub use crate::location::ProjectRoot;
use crate::editor_tabs::EditorTabs;
use crate::location::FileLocation;

/// 新しく開くプロジェクトでターミナルを起動するか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    /// フォルダを開いたとき。既定のシェルのターミナルを 1 枚起動する。
    WithTerminal,
    /// ファイルを開いたとき。ターミナルは起動しない（設計書 §5.1）。
    EditorOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProjectId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaneId(u64);

impl PaneId {
    /// FUDE_TERMINAL_ID などに使う番号。
    pub fn raw(&self) -> u64 {
        self.0
    }
}

/// ターミナルで動かすシェルの種類。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellKind {
    Native,
    Wsl { distro: String },
}

impl ShellKind {
    /// タブやペインに表示する短い名前。
    pub fn label(&self) -> String {
        match self {
            ShellKind::Native if cfg!(target_os = "macos") => {
                native_label_from_shell(std::env::var("SHELL").ok().as_deref())
            }
            ShellKind::Native => "pwsh".into(),
            ShellKind::Wsl { distro } => format!("wsl·{distro}"),
        }
    }
}

pub struct Pane {
    pub id: PaneId,
    pub kind: ShellKind,
    /// 端末が OSC で設定した題名。
    pub title: Option<String>,
}

/// ターミナルのタブ。ペインを 1 つか 2 つ（左右の分割）持つ。
pub struct Tab {
    pub id: TabId,
    pub panes: Vec<Pane>,
    pub focused: usize,
}

impl Tab {
    /// フォーカス中のペインの名前。端末が題名を設定していればそれ、無ければシェルの種類。
    pub fn title(&self) -> String {
        let pane = &self.panes[self.focused];
        pane.title.as_deref().and_then(display_title).unwrap_or_else(|| pane.kind.label())
    }
}

pub struct Project {
    pub id: ProjectId,
    pub root: ProjectRoot,
    pub name: String,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<TabId>,
    /// 既定のシェルの上書き（WSL から開いた /mnt の下のフォルダなど）。
    pub shell: Option<ShellKind>,
    /// エディタ列のファイルのタブの並び。ビューが更新する。
    pub editor: EditorTabs,
}

impl Project {
    /// 新しいターミナルの既定のシェル。上書きがあればそれ、無ければ root から決まるもの。
    pub fn default_shell(&self) -> ShellKind {
        self.shell.clone().unwrap_or_else(|| self.root.default_shell())
    }

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
        let pane = Pane { id: PaneId(self.alloc()), kind, title: None };
        Tab { id, panes: vec![pane], focused: 0 }
    }

    /// root は呼び出し側で正規化済みとする。同じ root があればそれを選び、その id を返す。
    /// 新しいプロジェクトには既定のシェル（root.default_shell()）のタブを 1 つ作る。
    pub fn open_project(&mut self, root: ProjectRoot) -> ProjectId {
        self.open_project_with(root, None, OpenMode::WithTerminal)
    }

    /// open_project と同じ。新しく作るときは shell を既定のシェルの上書きとして持ち、最初のタブもそのシェルにする。
    /// mode が EditorOnly ならタブを作らない。すでに開いていれば切り替えるだけ（shell もタブも変えない）。
    pub fn open_project_with(&mut self, root: ProjectRoot, shell: Option<ShellKind>, mode: OpenMode) -> ProjectId {
        if let Some(p) = self.projects.iter().find(|p| p.root.same_place(&root)) {
            let id = p.id;
            self.active = Some(id);
            return id;
        }
        let id = ProjectId(self.alloc());
        let tabs = match mode {
            OpenMode::WithTerminal => vec![self.make_tab(shell.clone().unwrap_or_else(|| root.default_shell()))],
            OpenMode::EditorOnly => Vec::new(),
        };
        let name = root.name();
        let active_tab = tabs.first().map(|t| t.id);
        self.projects.push(Project { id, root, name, active_tab, tabs, shell, editor: EditorTabs::default() });
        self.active = Some(id);
        id
    }

    /// 復元用。種類の並び（タブごとのペインと focused）からタブを作り、プロジェクトを末尾に足す。
    /// 選択は変えない。tabs が空ならタブを作らない。active_tab と focused は範囲に収める。
    pub(crate) fn push_restored(
        &mut self,
        root: ProjectRoot,
        tabs: &[(Vec<ShellKind>, usize)],
        active_tab: Option<usize>,
        shell: Option<ShellKind>,
        editor: EditorTabs,
    ) -> ProjectId {
        let id = ProjectId(self.alloc());
        let mut built = Vec::new();
        for (kinds, focused) in tabs {
            let panes = kinds.iter().map(|k| Pane { id: PaneId(self.alloc()), kind: k.clone(), title: None }).collect();
            let focused = if *focused < kinds.len() { *focused } else { 0 };
            built.push(Tab { id: TabId(self.alloc()), panes, focused });
        }
        let active = built.get(active_tab.unwrap_or(0)).or(built.first()).map(|t| t.id);
        let name = root.name();
        self.projects.push(Project { id, root, name, tabs: built, active_tab: active, shell, editor });
        id
    }

    /// 復元用。位置で選ぶ（範囲外なら先頭、プロジェクトが無ければ未選択）。
    pub(crate) fn select_restored(&mut self, index: Option<usize>) {
        let i = index.filter(|&i| i < self.projects.len()).unwrap_or(0);
        self.active = self.projects.get(i).map(|p| p.id);
    }

    pub fn editor_mut(&mut self, id: ProjectId) -> Option<&mut EditorTabs> {
        self.projects.iter_mut().find(|p| p.id == id).map(|p| &mut p.editor)
    }

    /// file を含むプロジェクトのうち、ルートが最も深いもの（相対パスが最も短いもの）と、その相対パス。
    pub fn project_for_file(&self, file: &FileLocation) -> Option<(ProjectId, String)> {
        self.projects
            .iter()
            .filter_map(|p| file.rel_within(&p.root).map(|rel| (p.id, rel)))
            .min_by_key(|(_, rel)| rel.split('/').filter(|s| !s.is_empty()).count())
    }

    pub fn activate_project(&mut self, id: ProjectId) {
        if self.projects.iter().any(|p| p.id == id) {
            self.active = Some(id);
        }
    }

    /// プロジェクトを取り除く。選択中なら右隣、無ければ左隣、無ければ未選択にする。
    pub fn close_project(&mut self, id: ProjectId) {
        let Some(i) = self.projects.iter().position(|p| p.id == id) else { return };
        self.projects.remove(i);
        if self.active == Some(id) {
            let next = if i < self.projects.len() { Some(i) } else { i.checked_sub(1) };
            self.active = next.map(|n| self.projects[n].id);
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
        if let Some(p) = self.active_project_mut() {
            remove_tab(p, id);
        }
    }

    fn find_pane_mut(&mut self, pane: PaneId) -> Option<(&mut Project, usize, usize)> {
        self.projects.iter_mut().find_map(|p| {
            let (ti, pi) = p
                .tabs
                .iter()
                .enumerate()
                .find_map(|(ti, t)| t.panes.iter().position(|x| x.id == pane).map(|pi| (ti, pi)))?;
            Some((p, ti, pi))
        })
    }

    /// どのプロジェクトのペインでもよい。見つからなければ何もしない。
    pub fn set_pane_title(&mut self, pane: PaneId, title: Option<String>) {
        if let Some((p, ti, pi)) = self.find_pane_mut(pane) {
            p.tabs[ti].panes[pi].title = title;
        }
    }

    /// ペインを含むタブの focused をそのペインにする。
    pub fn focus_pane(&mut self, pane: PaneId) {
        if let Some((p, ti, pi)) = self.find_pane_mut(pane) {
            p.tabs[ti].focused = pi;
        }
    }

    /// ペインを取り除く。タブが空になったらタブも閉じ、閉じたタブの id を返す。
    pub fn close_pane(&mut self, pane: PaneId) -> Option<TabId> {
        let (p, ti, pi) = self.find_pane_mut(pane)?;
        let tab = &mut p.tabs[ti];
        tab.panes.remove(pi);
        if tab.panes.is_empty() {
            let id = tab.id;
            remove_tab(p, id);
            return Some(id);
        }
        tab.focused = tab.focused.min(tab.panes.len() - 1);
        None
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
        let pane = Pane { id: PaneId(self.alloc()), kind, title: None };
        if let Some(p) = self.active_project_mut()
            && let Some(i) = p.active_index()
        {
            let tab = &mut p.tabs[i];
            tab.panes.push(pane);
            tab.focused = tab.panes.len() - 1;
        }
    }
}

/// タブを取り除く。選択中のタブなら、右隣、無ければ左隣を選ぶ。
fn remove_tab(p: &mut Project, id: TabId) {
    let Some(i) = p.tabs.iter().position(|t| t.id == id) else { return };
    p.tabs.remove(i);
    if p.active_tab == Some(id) {
        let next = if i < p.tabs.len() { Some(i) } else { i.checked_sub(1) };
        p.active_tab = next.map(|n| p.tabs[n].id);
    }
}

/// OSC の題名を表示用に整える。前後の空白を除き、空なら None。
/// 絶対パスなら、ファイル名から拡張子を除いたものにする（pwsh が自分の exe のパスを題名にするため）。
pub fn display_title(raw: &str) -> Option<String> {
    let title = raw.trim();
    if title.is_empty() {
        return None;
    }
    if title.contains('\\') || title.starts_with('/') {
        let name = title.rsplit(['\\', '/']).next().unwrap_or(title);
        let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        if !stem.is_empty() {
            return Some(stem.to_string());
        }
    }
    Some(title.to_string())
}

/// macOS のネイティブシェルの名前。ログインシェル（$SHELL）のファイル名、無ければ zsh。
/// 起動するシェル（fude-term の spawn_spec）と同じ規則にする。
pub fn native_label_from_shell(shell: Option<&str>) -> String {
    shell
        .filter(|s| !s.is_empty())
        .and_then(|s| Path::new(s).file_name())
        .map_or_else(|| "zsh".to_string(), |n| n.to_string_lossy().into_owned())
}

/// プロジェクトのアイコンの色を、root から決める（6 色の中から）。
/// host_path の文字列の FNV-1a を使い、実行のたびに同じ値になる。
pub fn project_color_index(root: &ProjectRoot) -> usize {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root.host_path().to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (hash % 6) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_tabs::EditorItem;
    use crate::location::FileLocation;
    use crate::panel::FileMode;
    use std::path::PathBuf;

    fn ws_with(roots: &[&str]) -> Workspace {
        let mut ws = Workspace::default();
        for root in roots {
            ws.open_project(local(root));
        }
        ws
    }

    fn local(root: &str) -> ProjectRoot {
        ProjectRoot::Local(PathBuf::from(root))
    }

    fn active(ws: &Workspace) -> &Project {
        ws.active_project().expect("選択中のプロジェクトがある")
    }

    fn tab_ids(ws: &Workspace) -> Vec<TabId> {
        active(ws).tabs.iter().map(|t| t.id).collect()
    }

    #[test]
    fn editor_only_project_has_no_tabs() {
        let mut ws = Workspace::default();
        let id = ws.open_project_with(local("/w/a"), None, OpenMode::EditorOnly);
        assert!(active(&ws).tabs.is_empty());
        assert_eq!(active(&ws).active_tab, None);
        assert_eq!(ws.open_project_with(local("/w/a"), None, OpenMode::WithTerminal), id);
        assert!(active(&ws).tabs.is_empty());
    }

    #[test]
    fn project_for_file_picks_deepest_root() {
        let ws = ws_with(&["/w", "/w/a"]);
        let f = FileLocation::Local(PathBuf::from("/w/a/src/x.rs"));
        let (id, rel) = ws.project_for_file(&f).unwrap();
        assert_eq!(ws.projects().iter().find(|p| p.id == id).unwrap().root, local("/w/a"));
        assert_eq!(rel, "src/x.rs");
        assert!(ws.project_for_file(&FileLocation::Local(PathBuf::from("/x/y.rs"))).is_none());
    }

    #[test]
    fn editor_mut_opens_and_closes_tabs() {
        let mut ws = ws_with(&["/w/a"]);
        let id = active(&ws).id;
        ws.editor_mut(id).unwrap().open(EditorItem { rel: "src/a.rs".into(), mode: FileMode::Full });
        assert_eq!(active(&ws).editor.active().unwrap().rel, "src/a.rs");
        ws.editor_mut(id).unwrap().close(0);
        assert!(active(&ws).editor.is_empty());
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
        let a = ws.open_project(local("/w/a"));
        ws.open_project(local("/w/b"));
        let again = ws.open_project(local("/w/a"));
        assert_eq!(ws.projects().len(), 2);
        assert_eq!(again, a);
        assert_eq!(active(&ws).id, a);
    }

    #[test]
    fn open_wsl_project_uses_wsl_shell() {
        let mut ws = Workspace::default();
        ws.open_project(ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x".into() });
        let p = active(&ws);
        assert_eq!(p.name, "x");
        assert_eq!(p.tabs[0].panes[0].kind, ShellKind::Wsl { distro: "Ubuntu".into() });
        assert_eq!(p.default_shell(), ShellKind::Wsl { distro: "Ubuntu".into() });
    }

    #[test]
    fn close_active_project_selects_right_neighbor() {
        let mut ws = ws_with(&["/w/a", "/w/b", "/w/c"]);
        let ids: Vec<_> = ws.projects().iter().map(|p| p.id).collect();
        ws.activate_project(ids[1]);
        ws.close_project(ids[1]);
        assert_eq!(ws.projects().len(), 2);
        assert_eq!(active(&ws).id, ids[2]);
        ws.close_project(ids[2]);
        assert_eq!(active(&ws).id, ids[0]);
    }

    #[test]
    fn close_last_project_leaves_none() {
        let mut ws = ws_with(&["/w/a"]);
        let id = ws.projects()[0].id;
        ws.close_project(id);
        assert!(ws.active_project().is_none());
        assert!(ws.projects().is_empty());
    }

    #[test]
    fn close_inactive_project_keeps_selection() {
        let mut ws = ws_with(&["/w/a", "/w/b"]);
        let (a, b) = (ws.projects()[0].id, ws.projects()[1].id);
        ws.close_project(a);
        assert_eq!(active(&ws).id, b);
    }

    #[test]
    fn distro_case_does_not_duplicate_project() {
        let mut ws = Workspace::default();
        let a = ws.open_project(ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x".into() });
        let again = ws.open_project(ProjectRoot::Wsl { distro: "ubuntu".into(), path: "/home/x".into() });
        assert_eq!(again, a);
        assert_eq!(ws.projects().len(), 1);
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

    fn pane_ids(ws: &Workspace) -> Vec<PaneId> {
        active(ws).active_tab().unwrap().panes.iter().map(|p| p.id).collect()
    }

    #[test]
    fn close_pane_in_split_keeps_tab() {
        let mut ws = ws_with(&["/w/a"]);
        ws.split_active_tab();
        let panes = pane_ids(&ws);
        assert_eq!(ws.close_pane(panes[1]), None);
        let tab = active(&ws).active_tab().unwrap();
        assert_eq!(tab.panes.len(), 1);
        assert_eq!(tab.focused, 0);
    }

    #[test]
    fn close_last_pane_closes_tab_and_selects_neighbor() {
        let mut ws = ws_with(&["/w/a"]);
        ws.new_tab(ShellKind::Native);
        ws.new_tab(ShellKind::Native);
        let ids = tab_ids(&ws);
        ws.activate_tab(ids[1]);
        let pane = pane_ids(&ws)[0];
        assert_eq!(ws.close_pane(pane), Some(ids[1]));
        assert_eq!(active(&ws).tabs.len(), 2);
        assert_eq!(active(&ws).active_tab, Some(ids[2]));
    }

    #[test]
    fn close_pane_in_inactive_project() {
        let mut ws = ws_with(&["/w/a", "/w/b"]);
        let a = &ws.projects()[0];
        let (a_id, pane) = (a.id, a.tabs[0].panes[0].id);
        let b_id = active(&ws).id;
        ws.close_pane(pane);
        assert!(ws.projects().iter().find(|p| p.id == a_id).unwrap().tabs.is_empty());
        assert_eq!(active(&ws).id, b_id);
    }

    #[test]
    fn focus_pane_sets_focused() {
        let mut ws = ws_with(&["/w/a"]);
        ws.split_active_tab();
        let panes = pane_ids(&ws);
        ws.focus_pane(panes[0]);
        assert_eq!(active(&ws).active_tab().unwrap().focused, 0);
    }

    #[test]
    fn pane_title_overrides_label() {
        let mut ws = ws_with(&["/w/a"]);
        let pane = pane_ids(&ws)[0];
        ws.set_pane_title(pane, Some("✳ Claude Code".into()));
        assert_eq!(active(&ws).active_tab().unwrap().title(), "✳ Claude Code");
        ws.set_pane_title(pane, None);
        assert_eq!(active(&ws).active_tab().unwrap().title(), ShellKind::Native.label());
    }

    #[test]
    fn display_title_rules() {
        assert_eq!(display_title("C:\\Program Files\\PowerShell\\7\\pwsh.exe").as_deref(), Some("pwsh"));
        assert_eq!(display_title("/usr/bin/zsh").as_deref(), Some("zsh"));
        assert_eq!(display_title(" ✳ Claude Code ").as_deref(), Some("✳ Claude Code"));
        assert_eq!(display_title("  "), None);
    }

    #[test]
    fn mac_native_label_follows_login_shell() {
        assert_eq!(native_label_from_shell(Some("/opt/homebrew/bin/fish")), "fish");
        assert_eq!(native_label_from_shell(Some("/bin/zsh")), "zsh");
        assert_eq!(native_label_from_shell(Some("")), "zsh");
        assert_eq!(native_label_from_shell(None), "zsh");
    }

    #[test]
    fn wsl_label() {
        let kind = ShellKind::Wsl { distro: "Ubuntu".into() };
        assert_eq!(kind.label(), "wsl·Ubuntu");
    }

    #[test]
    fn project_color_index_is_stable() {
        let root = local("/w/a");
        assert_eq!(project_color_index(&root), project_color_index(&root));
        let values: std::collections::HashSet<usize> = ('a'..='z')
            .map(|c| project_color_index(&local(&format!("/w/{c}"))))
            .collect();
        assert!(values.iter().all(|&v| v < 6));
        assert!(values.len() >= 2);
    }

    #[test]
    fn open_with_shell_overrides_default() {
        let mut ws = Workspace::default();
        let ubuntu = ShellKind::Wsl { distro: "Ubuntu".into() };
        ws.open_project_with(local(r"C:\w"), Some(ubuntu.clone()), OpenMode::WithTerminal);
        let p = active(&ws);
        assert_eq!(p.tabs[0].panes[0].kind, ubuntu);
        assert_eq!(p.default_shell(), ubuntu);
    }

    #[test]
    fn reopen_keeps_original_shell() {
        let mut ws = Workspace::default();
        let ubuntu = ShellKind::Wsl { distro: "Ubuntu".into() };
        let a = ws.open_project_with(local(r"C:\w"), Some(ubuntu.clone()), OpenMode::WithTerminal);
        let again = ws.open_project_with(local(r"C:\w"), None, OpenMode::WithTerminal);
        assert_eq!(again, a);
        assert_eq!(ws.projects().len(), 1);
        assert_eq!(active(&ws).default_shell(), ubuntu);
    }
}
