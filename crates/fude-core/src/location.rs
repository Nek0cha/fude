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
        let Some(text) = path.to_str() else { return ProjectRoot::Local(path) };
        let Some(rest) = text.strip_prefix(r"\\") else { return ProjectRoot::Local(path) };
        let mut parts = rest.split('\\').filter(|s| !s.is_empty());
        let (Some(host), Some(distro)) = (parts.next(), parts.next()) else { return ProjectRoot::Local(path) };
        if !WSL_HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host)) {
            return ProjectRoot::Local(path);
        }
        let segments: Vec<&str> = parts.collect();
        if let ["mnt", drive, tail @ ..] = segments.as_slice()
            && drive.len() == 1
            && drive.as_bytes()[0].is_ascii_alphabetic()
        {
            let mut local = format!("{}:\\", drive.to_ascii_uppercase());
            local.push_str(&tail.join("\\"));
            return ProjectRoot::Local(PathBuf::from(local));
        }
        let path = format!("/{}", segments.join("/"));
        ProjectRoot::Wsl { distro: distro.to_string(), path }
    }

    /// WSL の中の絶対パスから作る。`/mnt/<英字 1 文字>` とその下は Local(`<大文字>:\…`)、それ以外は Wsl。
    /// 末尾の "/" は除く（"/" だけはそのまま）。
    pub fn from_wsl_path(distro: &str, path: &str) -> ProjectRoot {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if let ["mnt", drive, tail @ ..] = segments.as_slice()
            && drive.len() == 1
            && drive.as_bytes()[0].is_ascii_alphabetic()
        {
            let mut local = format!("{}:\\", drive.to_ascii_uppercase());
            local.push_str(&tail.join("\\"));
            return ProjectRoot::Local(PathBuf::from(local));
        }
        ProjectRoot::Wsl { distro: distro.to_string(), path: format!("/{}", segments.join("/")) }
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
}
