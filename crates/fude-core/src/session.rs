//! 終了時の状態（セッション）の保存と復元。

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::keymap::Platform;
use crate::workspace::{ProjectRoot, ShellKind, Workspace};

pub const SESSION_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SessionData {
    pub version: u32,
    pub window: Option<WindowGeom>,
    /// 選択中のプロジェクトの位置。
    pub active: Option<usize>,
    pub projects: Vec<ProjectState>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectState {
    pub root: ProjectRoot,
    pub tabs: Vec<TabState>,
    pub active_tab: Option<usize>,
    /// 既定のシェルの上書き。計画4の形式には無いため、無ければ None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellKind>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TabState {
    pub panes: Vec<ShellKind>,
    pub focused: usize,
}

/// 論理ピクセル。maximized のときの x..height は元に戻したときの大きさ。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WindowGeom {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl SessionData {
    pub fn capture(ws: &Workspace, window: Option<WindowGeom>) -> SessionData {
        let projects = ws
            .projects()
            .iter()
            .map(|p| ProjectState {
                root: p.root.clone(),
                tabs: p
                    .tabs
                    .iter()
                    .map(|t| TabState { panes: t.panes.iter().map(|x| x.kind.clone()).collect(), focused: t.focused })
                    .collect(),
                active_tab: p.active_tab.and_then(|id| p.tabs.iter().position(|t| t.id == id)),
                shell: p.shell.clone(),
            })
            .collect();
        let active = ws.active_project().and_then(|a| ws.projects().iter().position(|p| p.id == a.id));
        SessionData { version: SESSION_VERSION, window, active, projects }
    }

    /// JSON が壊れている、または version が SESSION_VERSION でなければ None。
    pub fn parse(text: &str) -> Option<SessionData> {
        serde_json::from_str::<SessionData>(text).ok().filter(|d| d.version == SESSION_VERSION)
    }

    /// 整形した JSON。
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("SessionData は必ず JSON にできる")
    }
}

impl Workspace {
    /// exists は Local のプロジェクトにだけ使う（WSL を起こさないため、Wsl は確かめずに戻す）。
    /// 存在しないプロジェクトと、ペインが空のタブは捨てる。タブが空になったプロジェクトには既定のシェルの
    /// タブを 1 つ作る。active、active_tab、focused は範囲に収める（範囲外なら先頭）。
    pub fn restore(data: &SessionData, exists: &dyn Fn(&Path) -> bool) -> Workspace {
        let mut ws = Workspace::default();
        let mut active = None;
        for (i, p) in data.projects.iter().enumerate() {
            if p.root.local_path().is_some_and(|path| !exists(path)) {
                continue;
            }
            // 同じ場所が 2 度書かれていたら、最初のものだけを使う。
            if ws.projects().iter().any(|q| q.root.same_place(&p.root)) {
                continue;
            }
            let mut tabs: Vec<(Vec<ShellKind>, usize)> =
                p.tabs.iter().filter(|t| !t.panes.is_empty()).map(|t| (t.panes.clone(), t.focused)).collect();
            if tabs.is_empty() {
                tabs.push((vec![p.shell.clone().unwrap_or_else(|| p.root.default_shell())], 0));
            }
            ws.push_restored(p.root.clone(), &tabs, p.active_tab, p.shell.clone());
            if data.active == Some(i) {
                active = Some(ws.projects().len() - 1);
            }
        }
        ws.select_restored(active);
        ws
    }
}

/// Windows: %APPDATA%\fude\session.json。macOS: $HOME/.config/fude/session.json（設定ファイルと同じ場所）。
/// FUDE_INSTANCE=x のときは session-x.json。環境変数が無ければ None。
pub fn session_path(os: Platform, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    data_file(os, env, "session")
}

/// fude のデータのディレクトリの `<stem>.json`（FUDE_INSTANCE=x なら `<stem>-x.json`）。
pub(crate) fn data_file(os: Platform, env: &dyn Fn(&str) -> Option<String>, stem: &str) -> Option<PathBuf> {
    let dir = match os {
        Platform::Windows => PathBuf::from(env("APPDATA").filter(|v| !v.is_empty())?).join("fude"),
        Platform::MacOs => PathBuf::from(env("HOME").filter(|v| !v.is_empty())?).join(".config").join("fude"),
    };
    let name = match env("FUDE_INSTANCE").filter(|v| !v.is_empty()) {
        Some(instance) => format!("{stem}-{instance}.json"),
        None => format!("{stem}.json"),
    };
    Some(dir.join(name))
}

/// 無い、読めない、壊れている → None（壊れているときだけ eprintln で理由を出す）。
pub fn load(path: &Path) -> Option<SessionData> {
    let text = std::fs::read_to_string(path).ok()?;
    let data = SessionData::parse(&text);
    if data.is_none() {
        eprintln!("セッションのファイルを読めないため、空の状態で始める: {}", path.display());
    }
    data
}

/// セッションを書く。
pub fn save(path: &Path, data: &SessionData) -> io::Result<()> {
    write_atomic(path, &data.to_json())
}

/// 親のディレクトリを作り、同じ場所の `<name>.tmp` に書いてから rename する。
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// ウィンドウの上端から 40px の帯のうち、幅 100px 以上が、どれかのディスプレイ（x, y, width, height）に入っていれば真。
pub fn window_is_visible(win: &WindowGeom, displays: &[(f32, f32, f32, f32)]) -> bool {
    const BAND_H: f32 = 40.;
    const MIN_W: f32 = 100.;
    displays.iter().any(|&(dx, dy, dw, dh)| {
        let w = (win.x + win.width).min(dx + dw) - win.x.max(dx);
        let h = (win.y + BAND_H).min(dy + dh) - win.y.max(dy);
        w >= MIN_W && h > 0.
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Platform;
    use crate::workspace::{ProjectRoot, ShellKind, Workspace};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn local(p: &str) -> ProjectRoot {
        ProjectRoot::Local(PathBuf::from(p))
    }

    fn wsl(distro: &str, path: &str) -> ProjectRoot {
        ProjectRoot::Wsl { distro: distro.into(), path: path.into() }
    }

    fn ubuntu() -> ShellKind {
        ShellKind::Wsl { distro: "Ubuntu".into() }
    }

    /// 比べやすいように、ワークスペースの形だけを取り出す。
    fn shape(ws: &Workspace) -> (Option<usize>, Vec<(ProjectRoot, Option<usize>, Vec<(Vec<ShellKind>, usize)>)>) {
        let data = SessionData::capture(ws, None);
        let projects = data
            .projects
            .into_iter()
            .map(|p| (p.root, p.active_tab, p.tabs.into_iter().map(|t| (t.panes, t.focused)).collect()))
            .collect();
        (data.active, projects)
    }

    #[test]
    fn roundtrip_restores_tabs_split_and_selection() {
        let mut ws = Workspace::default();
        ws.open_project(local("/w/a"));
        ws.new_tab(ubuntu());
        ws.split_active_tab();
        ws.new_tab(ShellKind::Native);
        let second = ws.active_project().unwrap().tabs[1].id;
        ws.activate_tab(second);
        let pane = ws.active_project().unwrap().tabs[1].panes[1].id;
        ws.focus_pane(pane);
        ws.open_project(wsl("Ubuntu", "/home/x"));
        ws.activate_project_at(0);

        let geom = WindowGeom { x: 10., y: 20., width: 1200., height: 780., maximized: true };
        let data = SessionData::capture(&ws, Some(geom.clone()));
        let parsed = SessionData::parse(&data.to_json()).expect("読み直せる");
        assert_eq!(parsed, data);
        assert_eq!(parsed.window, Some(geom));
        let restored = Workspace::restore(&parsed, &|_| true);
        assert_eq!(shape(&restored), shape(&ws));
        assert_eq!(restored.active_project().unwrap().root, local("/w/a"));
        let tab = restored.active_project().unwrap().active_tab().unwrap();
        assert_eq!(tab.panes.len(), 2);
        assert_eq!(tab.focused, 1);
    }

    #[test]
    fn json_shape_is_stable() {
        let text = r#"{"version":1,"window":{"x":10.0,"y":20.0,"width":1200.0,"height":780.0,"maximized":false},"active":0,
            "projects":[{"root":{"local":"C:\\w\\a"},"active_tab":0,"tabs":[{"panes":["native",{"wsl":{"distro":"Ubuntu"}}],"focused":1}]},
                        {"root":{"wsl":{"distro":"Ubuntu","path":"/home/x"}},"active_tab":null,"tabs":[]}]}"#;
        let expected = SessionData {
            version: 1,
            window: Some(WindowGeom { x: 10., y: 20., width: 1200., height: 780., maximized: false }),
            active: Some(0),
            projects: vec![
                ProjectState {
                    root: local(r"C:\w\a"),
                    tabs: vec![TabState { panes: vec![ShellKind::Native, ubuntu()], focused: 1 }],
                    active_tab: Some(0),
                    shell: None,
                },
                ProjectState { root: wsl("Ubuntu", "/home/x"), tabs: vec![], active_tab: None, shell: None },
            ],
        };
        assert_eq!(SessionData::parse(text), Some(expected));
    }

    #[test]
    fn corrupt_or_unknown_version_is_none() {
        assert_eq!(SessionData::parse(""), None);
        assert_eq!(SessionData::parse("{"), None);
        assert_eq!(SessionData::parse(r#"{"version":2,"window":null,"active":null,"projects":[]}"#), None);
    }

    #[test]
    fn missing_local_root_is_skipped_wsl_is_kept() {
        let mut ws = Workspace::default();
        ws.open_project(local("/w/gone"));
        ws.open_project(wsl("Ubuntu", "/home/x"));
        let data = SessionData::capture(&ws, None);
        let restored = Workspace::restore(&data, &|p: &Path| {
            assert!(!p.to_string_lossy().contains("wsl"), "WSL のパスを確かめてはいけない: {}", p.display());
            false
        });
        let roots: Vec<_> = restored.projects().iter().map(|p| p.root.clone()).collect();
        assert_eq!(roots, vec![wsl("Ubuntu", "/home/x")]);
        assert_eq!(restored.active_project().unwrap().root, wsl("Ubuntu", "/home/x"));
    }

    #[test]
    fn indices_are_clamped() {
        let data = SessionData {
            version: 1,
            window: None,
            active: Some(9),
            projects: vec![ProjectState {
                root: local("/w/a"),
                tabs: vec![TabState { panes: vec![ShellKind::Native], focused: 5 }],
                active_tab: Some(7),
                shell: None,
            }],
        };
        let ws = Workspace::restore(&data, &|_| true);
        let p = ws.active_project().expect("先頭が選ばれる");
        assert_eq!(p.active_tab, Some(p.tabs[0].id));
        assert_eq!(p.tabs[0].focused, 0);
    }

    #[test]
    fn empty_tabs_get_default_shell_tab() {
        let data = SessionData {
            version: 1,
            window: None,
            active: None,
            projects: vec![ProjectState {
                root: wsl("Ubuntu", "/home/x"),
                tabs: vec![TabState { panes: vec![], focused: 0 }],
                active_tab: None,
                shell: None,
            }],
        };
        let ws = Workspace::restore(&data, &|_| true);
        let p = ws.active_project().expect("先頭が選ばれる");
        assert_eq!(p.tabs.len(), 1);
        assert_eq!(p.tabs[0].panes[0].kind, ubuntu());
        assert_eq!(p.active_tab, Some(p.tabs[0].id));
    }

    #[test]
    fn shell_override_roundtrips() {
        let mut ws = Workspace::default();
        ws.open_project_with(local(r"C:\w"), Some(ubuntu()));
        let data = SessionData::capture(&ws, None);
        assert_eq!(data.projects[0].shell, Some(ubuntu()));
        let parsed = SessionData::parse(&data.to_json()).expect("読み直せる");
        let restored = Workspace::restore(&parsed, &|_| true);
        assert_eq!(restored.active_project().unwrap().default_shell(), ubuntu());
    }

    #[test]
    fn session_without_shell_field_parses() {
        let text = r#"{"version":1,"window":null,"active":0,
            "projects":[{"root":{"local":"C:\\w"},"active_tab":0,"tabs":[{"panes":["native"],"focused":0}]}]}"#;
        let data = SessionData::parse(text).expect("計画4の形式も読める");
        assert_eq!(data.projects[0].shell, None);
    }

    fn path_with(os: Platform, vars: &[(&str, &str)]) -> Option<PathBuf> {
        let map: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        session_path(os, &|k| map.get(k).cloned())
    }

    #[test]
    fn session_path_per_os_and_instance() {
        assert_eq!(
            path_with(Platform::Windows, &[("APPDATA", r"C:\u\AppData\Roaming")]),
            Some(Path::new(r"C:\u\AppData\Roaming").join("fude").join("session.json"))
        );
        assert_eq!(
            path_with(Platform::MacOs, &[("HOME", "/Users/u")]),
            Some(Path::new("/Users/u").join(".config").join("fude").join("session.json"))
        );
        assert_eq!(
            path_with(Platform::MacOs, &[("HOME", "/Users/u"), ("FUDE_INSTANCE", "dev")]),
            Some(Path::new("/Users/u").join(".config").join("fude").join("session-dev.json"))
        );
        assert_eq!(path_with(Platform::Windows, &[("HOME", "/Users/u")]), None);
    }

    #[test]
    fn save_is_atomic_and_creates_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("session.json");
        let mut ws = Workspace::default();
        ws.open_project(local("/w/a"));
        let data = SessionData::capture(&ws, None);
        save(&path, &data).unwrap();
        assert_eq!(load(&path), Some(data));
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("session.json")]);
        assert_eq!(load(&dir.path().join("none.json")), None);
    }

    #[test]
    fn window_visibility() {
        let geom = |x: f32, y: f32| WindowGeom { x, y, width: 1200., height: 780., maximized: false };
        let one = [(0., 0., 1920., 1080.)];
        assert!(window_is_visible(&geom(100., 100.), &one));
        assert!(!window_is_visible(&geom(3000., 100.), &one));
        assert!(!window_is_visible(&geom(-1150., 100.), &one));
        let two = [(0., 0., 1920., 1080.), (1920., 0., 1920., 1080.)];
        assert!(window_is_visible(&geom(2000., 100.), &two));
        assert!(!window_is_visible(&geom(100., 100.), &[]));
    }
}
