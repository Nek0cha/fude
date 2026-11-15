//! git で外したもの（gitignore など）の扱い（計画8d）。ツリー、絞り込み、⌘P、監視では外したものも出すが、
//! 重いフォルダ（HIDDEN_DIRS）は、外されているとき（git のリポジトリでなければ名前だけで）出さず、歩かない。
//! 「外した」は ignore クレートが外すものすべて（.gitignore、.git/info/exclude、グローバルな gitignore、
//! .ignore、.rgignore）。

use std::collections::HashSet;
use std::ffi::OsString;
use std::io;
use std::path::Path;

use ignore::WalkBuilder;

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

pub fn is_hidden_dir(name: &str) -> bool {
    HIDDEN_DIRS.contains(&name)
}

/// root かその祖先に .git（フォルダかファイル）があるか。無ければ .gitignore は効かない。
pub fn in_git_repo(root: &Path) -> bool {
    root.ancestors().any(|d| d.join(".git").exists())
}

/// フォルダを出すか。隠す名前のフォルダは、外されているか、git のリポジトリでなければ出さない。
pub(crate) fn hides_dir(name: &str, ignored: bool, in_repo: bool) -> bool {
    is_hidden_dir(name) && (ignored || !in_repo)
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
        assert!(is_hidden_dir("node_modules"));
        assert!(is_hidden_dir("Pods"));
        assert!(!is_hidden_dir("pods"));
        assert!(!is_hidden_dir("docs"));
        assert_eq!(HIDDEN_DIRS.len(), 22);
    }

    #[test]
    fn hides_only_ignored_or_outside_git() {
        assert!(hides_dir("vendor", true, true));
        assert!(!hides_dir("vendor", false, true));
        assert!(hides_dir("vendor", false, false));
        assert!(!hides_dir("docs", true, true));
    }
}
