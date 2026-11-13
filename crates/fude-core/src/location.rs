//! プロジェクトの場所。このマシンのフォルダか、WSL のフォルダか。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::workspace::ShellKind;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRoot {
    /// このマシンのフォルダ（Windows のパス、または macOS のパス）。
    Local(PathBuf),
    /// WSL のフォルダ。path は Linux の絶対パス（"/" 始まり、末尾の "/" なし。直下は "/"）。
    Wsl { distro: String, path: String },
}

/// WSL のフォルダを指す UNC のホスト名。
const WSL_HOSTS: [&str; 2] = ["wsl.localhost", "wsl$"];

impl ProjectRoot {
    /// 正規化済みのパス（`\\?\` は simplify_verbatim で外したもの）から作る。
    /// `\\wsl.localhost\<distro>\…` と `\\wsl$\<distro>\…`（ホスト名の大文字小文字は問わない）は Wsl。
    /// ただし `\mnt\<英字 1 文字>\…` の下は Local(`<英字の大文字>:\…`)。それ以外は Local。
    pub fn from_host_path(path: PathBuf) -> ProjectRoot {
        match parse_host_path(path) {
            Place::Local(p) => ProjectRoot::Local(p),
            Place::Wsl { distro, path } => ProjectRoot::Wsl { distro, path },
        }
    }

    /// WSL の中の絶対パスから作る。`/mnt/<英字 1 文字>` とその下は Local(`<大文字>:\…`)、それ以外は Wsl。
    /// 末尾の "/" は除く（"/" だけはそのまま）。
    pub fn from_wsl_path(distro: &str, path: &str) -> ProjectRoot {
        match parse_wsl_path(distro, path) {
            Place::Local(p) => ProjectRoot::Local(p),
            Place::Wsl { distro, path } => ProjectRoot::Wsl { distro, path },
        }
    }

    /// このプロセスから開けるパス。Wsl は `\\wsl.localhost\<distro>\<path の / を \ にしたもの>`。
    pub fn host_path(&self) -> PathBuf {
        match self {
            ProjectRoot::Local(p) => p.clone(),
            ProjectRoot::Wsl { distro, path } => {
                PathBuf::from(format!(r"\\wsl.localhost\{distro}\{}", path.trim_start_matches('/').replace('/', "\\")))
            }
        }
    }

    /// レールの頭文字とピルに使う名前。最後の要素。Wsl の "/" はディストリビューション名。
    pub fn name(&self) -> String {
        match self {
            ProjectRoot::Local(p) => {
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
            }
            ProjectRoot::Wsl { distro, path } => match path.rsplit('/').next() {
                Some(last) if !last.is_empty() => last.to_string(),
                _ => distro.clone(),
            },
        }
    }

    /// Wsl なら ShellKind::Wsl { distro }、Local なら ShellKind::Native。
    pub fn default_shell(&self) -> ShellKind {
        match self {
            ProjectRoot::Local(_) => ShellKind::Native,
            ProjectRoot::Wsl { distro, .. } => ShellKind::Wsl { distro: distro.clone() },
        }
    }

    /// 「パスをコピー」で渡す文字列。Local は host_path、Wsl は Linux のパス。
    pub fn copy_text(&self) -> String {
        match self {
            ProjectRoot::Local(p) => p.display().to_string(),
            ProjectRoot::Wsl { path, .. } => path.clone(),
        }
    }

    /// 同じフォルダか。WSL のディストリビューション名は大文字小文字を区別しない（wsl.exe と同じ）。
    pub fn same_place(&self, other: &ProjectRoot) -> bool {
        match (self, other) {
            (ProjectRoot::Wsl { distro: a, path: p }, ProjectRoot::Wsl { distro: b, path: q }) => {
                a.eq_ignore_ascii_case(b) && p == q
            }
            _ => self == other,
        }
    }

    /// Local のときのパス。
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            ProjectRoot::Local(p) => Some(p),
            ProjectRoot::Wsl { .. } => None,
        }
    }
}

/// パスを分けた結果。ProjectRoot と FileLocation で同じ規則を使うため。
enum Place {
    Local(PathBuf),
    Wsl { distro: String, path: String },
}

fn parse_host_path(path: PathBuf) -> Place {
    let Some(text) = path.to_str() else { return Place::Local(path) };
    let Some(rest) = text.strip_prefix(r"\\") else { return Place::Local(path) };
    let mut parts = rest.split('\\').filter(|s| !s.is_empty());
    let (Some(host), Some(distro)) = (parts.next(), parts.next()) else { return Place::Local(path) };
    if !WSL_HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        return Place::Local(path);
    }
    let segments: Vec<&str> = parts.collect();
    from_segments(distro, &segments)
}

fn parse_wsl_path(distro: &str, path: &str) -> Place {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    from_segments(distro, &segments)
}

/// WSL の中の要素の並びから。`mnt/<英字 1 文字>/…` は Windows のドライブ。
fn from_segments(distro: &str, segments: &[&str]) -> Place {
    if let ["mnt", drive, tail @ ..] = segments
        && drive.len() == 1
        && drive.as_bytes()[0].is_ascii_alphabetic()
    {
        let mut local = format!("{}:\\", drive.to_ascii_uppercase());
        local.push_str(&tail.join("\\"));
        return Place::Local(PathBuf::from(local));
    }
    Place::Wsl { distro: distro.to_string(), path: format!("/{}", segments.join("/")) }
}

/// 開くファイルの場所。このマシンのファイルか、WSL のファイルか。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileLocation {
    Local(PathBuf),
    /// path は Linux の絶対パス。
    Wsl { distro: String, path: String },
}

impl FileLocation {
    /// ProjectRoot::from_host_path と同じ規則。
    pub fn from_host_path(path: PathBuf) -> FileLocation {
        match parse_host_path(path) {
            Place::Local(p) => FileLocation::Local(p),
            Place::Wsl { distro, path } => FileLocation::Wsl { distro, path },
        }
    }

    /// ProjectRoot::from_wsl_path と同じ規則。
    pub fn from_wsl_path(distro: &str, path: &str) -> FileLocation {
        match parse_wsl_path(distro, path) {
            Place::Local(p) => FileLocation::Local(p),
            Place::Wsl { distro, path } => FileLocation::Wsl { distro, path },
        }
    }

    /// root の中なら、root からの相対パス（"/" 区切り。root 自身は ""）。要素ごとに比べる。
    /// Windows の Local は大文字小文字を区別しない。WSL はディストリビューション名だけ区別しない。
    pub fn rel_within(&self, root: &ProjectRoot) -> Option<String> {
        match (self, root) {
            (FileLocation::Local(file), ProjectRoot::Local(dir)) => {
                let file: Vec<String> = file.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                let dir: Vec<String> = dir.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                let same = |a: &String, b: &String| if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b };
                if file.len() < dir.len() || !dir.iter().zip(&file).all(|(d, f)| same(d, f)) {
                    return None;
                }
                Some(file[dir.len()..].join("/"))
            }
            (FileLocation::Wsl { distro: a, path: file }, ProjectRoot::Wsl { distro: b, path: dir }) => {
                if !a.eq_ignore_ascii_case(b) {
                    return None;
                }
                let file: Vec<&str> = file.split('/').filter(|s| !s.is_empty()).collect();
                let dir: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
                if file.len() < dir.len() || dir.iter().zip(&file).any(|(d, f)| d != f) {
                    return None;
                }
                Some(file[dir.len()..].join("/"))
            }
            _ => None,
        }
    }
}

/// ファイルの親から上へたどり、has_git_marker(dir) が真の最初の dir。無ければファイルの親。
/// has_git_marker は `.git`（ディレクトリかファイル）の有無を調べる関数を渡す。
pub fn find_project_root(file: &Path, has_git_marker: &dyn Fn(&Path) -> bool) -> PathBuf {
    let parent = file.parent().unwrap_or(file);
    parent.ancestors().find(|d| has_git_marker(d)).unwrap_or(parent).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ShellKind;
    use std::path::{Path, PathBuf};

    fn wsl(distro: &str, path: &str) -> ProjectRoot {
        ProjectRoot::Wsl { distro: distro.into(), path: path.into() }
    }

    fn from(p: &str) -> ProjectRoot {
        ProjectRoot::from_host_path(PathBuf::from(p))
    }

    #[test]
    fn wsl_localhost_unc_is_wsl_root() {
        assert_eq!(from(r"\\wsl.localhost\Ubuntu\home\neko\proj"), wsl("Ubuntu", "/home/neko/proj"));
    }

    #[test]
    fn wsl_dollar_and_case() {
        assert_eq!(from(r"\\WSL$\Ubuntu-24.04\home\x\"), wsl("Ubuntu-24.04", "/home/x"));
        assert_eq!(from(r"\\WSL.LOCALHOST\Ubuntu\home\x"), wsl("Ubuntu", "/home/x"));
    }

    #[test]
    fn distro_root() {
        assert_eq!(from(r"\\wsl.localhost\Ubuntu"), wsl("Ubuntu", "/"));
        assert_eq!(from(r"\\wsl.localhost\Ubuntu\"), wsl("Ubuntu", "/"));
        assert_eq!(wsl("Ubuntu", "/").name(), "Ubuntu");
    }

    #[test]
    fn mnt_drive_becomes_local() {
        assert_eq!(from(r"\\wsl.localhost\Ubuntu\mnt\c\Users\x"), ProjectRoot::Local(PathBuf::from(r"C:\Users\x")));
        assert_eq!(from(r"\\wsl.localhost\Ubuntu\mnt\c"), ProjectRoot::Local(PathBuf::from(r"C:\")));
        assert_eq!(from(r"\\wsl.localhost\Ubuntu\mnt\wsl\x"), wsl("Ubuntu", "/mnt/wsl/x"));
    }

    #[test]
    fn other_paths_are_local() {
        for p in [r"C:\w\a", r"\\server\share\x", "/Users/x"] {
            assert_eq!(from(p), ProjectRoot::Local(PathBuf::from(p)));
        }
    }

    #[test]
    fn from_wsl_path_rules() {
        let from = ProjectRoot::from_wsl_path;
        assert_eq!(from("Ubuntu", "/home/x/"), wsl("Ubuntu", "/home/x"));
        assert_eq!(from("Ubuntu", "/"), wsl("Ubuntu", "/"));
        assert_eq!(from("Ubuntu", "/mnt/c/Users/x"), ProjectRoot::Local(PathBuf::from(r"C:\Users\x")));
        assert_eq!(from("Ubuntu", "/mnt/d"), ProjectRoot::Local(PathBuf::from(r"D:\")));
        assert_eq!(from("Ubuntu", "/mnt/d/"), ProjectRoot::Local(PathBuf::from(r"D:\")));
        assert_eq!(from("Ubuntu", "/mnt/wsl/x"), wsl("Ubuntu", "/mnt/wsl/x"));
        assert_eq!(from("Ubuntu", "/home/x/my proj"), wsl("Ubuntu", "/home/x/my proj"));
    }

    #[test]
    fn same_place_ignores_distro_case() {
        assert!(wsl("Ubuntu", "/home/x").same_place(&wsl("UBUNTU", "/home/x")));
        assert!(!wsl("Ubuntu", "/home/x").same_place(&wsl("Ubuntu", "/home/X")));
        assert!(!wsl("Ubuntu", "/home/x").same_place(&wsl("Debian", "/home/x")));
    }

    #[test]
    fn host_path_of_wsl_root() {
        assert_eq!(wsl("Ubuntu", "/home/x").host_path(), Path::new(r"\\wsl.localhost\Ubuntu\home\x"));
        assert_eq!(wsl("Ubuntu", "/").host_path(), Path::new(r"\\wsl.localhost\Ubuntu\"));
    }

    #[test]
    fn default_shell_and_copy_text() {
        let w = wsl("Ubuntu", "/home/x");
        assert_eq!(w.default_shell(), ShellKind::Wsl { distro: "Ubuntu".into() });
        assert_eq!(w.copy_text(), "/home/x");
        assert_eq!(w.name(), "x");
        let l = ProjectRoot::Local(PathBuf::from(r"C:\w"));
        assert_eq!(l.default_shell(), ShellKind::Native);
        assert_eq!(l.copy_text(), r"C:\w");
    }

    fn local(p: &str) -> ProjectRoot {
        ProjectRoot::Local(PathBuf::from(p))
    }

    #[test]
    fn find_project_root_prefers_nearest_git() {
        let marks = ["/w/repo", "/w/repo/sub"];
        let has = |d: &Path| marks.iter().any(|m| Path::new(m) == d);
        assert_eq!(find_project_root(Path::new("/w/repo/sub/src/a.rs"), &has), Path::new("/w/repo/sub"));
        assert_eq!(find_project_root(Path::new("/w/repo/b.rs"), &has), Path::new("/w/repo"));
        assert_eq!(find_project_root(Path::new("/tmp/notes/x.md"), &has), Path::new("/tmp/notes"));
    }

    #[test]
    fn rel_within_rules() {
        let f = FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/x/p/src/a.rs".into() };
        assert_eq!(f.rel_within(&wsl("ubuntu", "/home/x/p")), Some("src/a.rs".into()));
        assert_eq!(f.rel_within(&wsl("Ubuntu", "/home/x/pp")), None);
        assert_eq!(f.rel_within(&wsl("Ubuntu", "/home/x/p/src/a.rs")), Some(String::new()));
        assert_eq!(f.rel_within(&wsl("Ubuntu", "/")), Some("home/x/p/src/a.rs".into()));
        assert_eq!(f.rel_within(&local("/home/x/p")), None);
        let l = FileLocation::Local(PathBuf::from("/w/a/b.rs"));
        assert_eq!(l.rel_within(&local("/w/a")), Some("b.rs".into()));
        assert_eq!(l.rel_within(&local("/w/ab")), None);
    }

    #[cfg(windows)]
    #[test]
    fn rel_within_windows_ignores_case() {
        let f = FileLocation::Local(PathBuf::from(r"C:\Work\App\src\main.rs"));
        assert_eq!(f.rel_within(&local(r"c:\work\app")), Some("src/main.rs".into()));
    }

    #[test]
    fn file_location_from_paths() {
        assert_eq!(
            FileLocation::from_host_path(PathBuf::from(r"\\wsl.localhost\Ubuntu\mnt\c\w\a.rs")),
            FileLocation::Local(PathBuf::from(r"C:\w\a.rs"))
        );
        assert_eq!(
            FileLocation::from_host_path(PathBuf::from(r"\\wsl$\Ubuntu\home\x\a.rs")),
            FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/x/a.rs".into() }
        );
        assert_eq!(
            FileLocation::from_wsl_path("Ubuntu", "/mnt/d/x/y.md"),
            FileLocation::Local(PathBuf::from(r"D:\x\y.md"))
        );
        assert_eq!(
            FileLocation::from_wsl_path("Ubuntu", "/home/x/y.md"),
            FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/x/y.md".into() }
        );
    }
}
