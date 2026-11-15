//! git で外したもの（gitignore など）の扱い（計画8d）。ツリー、絞り込み、⌘P、監視では外したものも出すが、
//! 重いフォルダ（HIDDEN_DIRS）は、外されているとき（git のリポジトリでなければ名前だけで）出さず、歩かない。
//! 「外した」は ignore クレートが外すものすべて（.gitignore、.git/info/exclude、グローバルな gitignore、
//! .ignore、.rgignore）。

use std::collections::HashSet;
use std::ffi::OsString;
use std::io;
use std::path::Path;

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::RelPath;

/// 外されていれば隠すフォルダの名前（完全一致、大文字小文字を区別、どの深さでも）。
pub const HIDDEN_DIRS: [&str; 22] = [
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    "build",
    ".next",
    ".gradle",
    "vendor",
    ".cache",
    "out",
    ".turbo",
    ".terraform",
    "Pods",
    ".mypy_cache",
    ".pytest_cache",
    "coverage",
    ".nuxt",
    ".svelte-kit",
    ".parcel-cache",
    ".angular",
];

/// 隠すフォルダの名前の一覧と、それをツリーに出すか（設定の tree.heavy_dirs と tree.show_heavy_dirs）。
/// 列挙（walk_files）と監視は show_in_tree に関係なく names を除く。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HeavyDirs {
    pub names: Vec<String>,
    pub show_in_tree: bool,
}

impl Default for HeavyDirs {
    fn default() -> Self {
        HeavyDirs { names: HIDDEN_DIRS.iter().map(|s| s.to_string()).collect(), show_in_tree: false }
    }
}

impl HeavyDirs {
    /// 設定の値から作る。names を書かなければ既定の一覧。
    pub fn from_settings(names: Option<Vec<String>>, show_in_tree: bool) -> HeavyDirs {
        HeavyDirs { names: names.unwrap_or_else(|| HeavyDirs::default().names), show_in_tree }
    }

    /// 隠す名前か（完全一致、大文字小文字を区別）。
    pub fn contains(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }
}

/// root かその祖先に .git（フォルダかファイル）があるか。無ければ .gitignore は効かない。
pub fn in_git_repo(root: &Path) -> bool {
    root.ancestors().any(|d| d.join(".git").exists())
}

/// フォルダを出すか。隠す名前のフォルダは、外されているか、git のリポジトリでなければ出さない。
pub(crate) fn hides_dir(heavy: &HeavyDirs, name: &str, ignored: bool, in_repo: bool) -> bool {
    heavy.contains(name) && (ignored || !in_repo)
}

/// dir の直下で、外されずに残る名前（隠しファイルは残す、.git は除く。上位のフォルダの規則も効かせる）。
pub(crate) fn kept_names(dir: &Path) -> io::Result<HashSet<OsString>> {
    let walker =
        WalkBuilder::new(dir).max_depth(Some(1)).hidden(false).parents(true).filter_entry(|e| e.file_name() != ".git").build();
    let mut names = HashSet::new();
    for entry in walker {
        let entry = entry.map_err(io::Error::other)?;
        if entry.depth() == 1 {
            names.insert(entry.file_name().to_os_string());
        }
    }
    Ok(names)
}

/// rel 自身か、その祖先が外されているか。各段を、その親の kept_names に無いかで確かめる。ルートは false。
pub(crate) fn is_ignored_path(root: &Path, rel: &RelPath) -> bool {
    let mut parent = root.to_path_buf();
    for part in rel.as_str().split('/').filter(|p| !p.is_empty()) {
        match kept_names(&parent) {
            Ok(kept) if kept.contains(std::ffi::OsStr::new(part)) => {}
            _ => return true,
        }
        parent.push(part);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_names_are_exact() {
        let d = HeavyDirs::default();
        assert!(d.contains("node_modules"));
        assert!(d.contains("Pods"));
        assert!(!d.contains("pods"));
        assert!(!d.contains("docs"));
        assert_eq!(HIDDEN_DIRS.len(), 22);
    }

    #[test]
    fn default_heavy_dirs_are_the_22() {
        let d = HeavyDirs::default();
        assert_eq!(d.names.len(), 22);
        assert!(!d.show_in_tree);
        assert_eq!(HeavyDirs::from_settings(None, true), HeavyDirs { show_in_tree: true, ..HeavyDirs::default() });
        assert_eq!(HeavyDirs::from_settings(Some(vec!["gen".into()]), false).names, vec!["gen".to_string()]);
    }

    #[test]
    fn hides_only_ignored_or_outside_git() {
        let d = HeavyDirs::default();
        assert!(hides_dir(&d, "vendor", true, true));
        assert!(!hides_dir(&d, "vendor", false, true));
        assert!(hides_dir(&d, "vendor", false, false));
        assert!(!hides_dir(&d, "docs", true, true));
    }
}
