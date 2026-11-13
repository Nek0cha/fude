//! .git/HEAD からブランチ名を読む。

use std::fs;
use std::path::{Path, PathBuf};

/// root/.git から HEAD を読み、ブランチ名を返す。detached HEAD ならハッシュの先頭 7 文字を返す。
/// Git の管理外、読めない、書式が不明なら None。
pub fn read_branch(root: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir(root)?.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(name) = head.strip_prefix("ref: refs/heads/") {
        return Some(name.to_owned());
    }
    let is_hash = matches!(head.len(), 40 | 64) && head.chars().all(|c| c.is_ascii_hexdigit());
    is_hash.then(|| head[..7].to_owned())
}

/// .git がディレクトリならそれを、`gitdir: <path>` を書いたファイルならその先を返す。
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let content = fs::read_to_string(&dot_git).ok()?;
    let target = Path::new(content.trim().strip_prefix("gitdir:")?.trim());
    Some(root.join(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo_with_head(head: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/HEAD"), head).unwrap();
        dir
    }

    #[test]
    fn branch_from_ref() {
        let dir = repo_with_head("ref: refs/heads/main\n");
        assert_eq!(read_branch(dir.path()).as_deref(), Some("main"));
    }

    #[test]
    fn nested_branch_name() {
        let dir = repo_with_head("ref: refs/heads/feat/window-layout\n");
        assert_eq!(read_branch(dir.path()).as_deref(), Some("feat/window-layout"));
    }

    #[test]
    fn detached_head() {
        let dir = repo_with_head("0123456789abcdef0123456789abcdef01234567\n");
        assert_eq!(read_branch(dir.path()).as_deref(), Some("0123456"));
    }

    #[test]
    fn no_git_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_branch(dir.path()), None);
    }

    #[test]
    fn gitdir_file_is_followed() {
        let gitdir = tempfile::tempdir().unwrap();
        fs::write(gitdir.path().join("HEAD"), "ref: refs/heads/wt\n").unwrap();
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".git"), format!("gitdir: {}\n", gitdir.path().display())).unwrap();
        assert_eq!(read_branch(root.path()).as_deref(), Some("wt"));
    }

    #[test]
    fn relative_gitdir_is_resolved_from_root() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("meta/wt")).unwrap();
        fs::write(root.path().join("meta/wt/HEAD"), "ref: refs/heads/rel\n").unwrap();
        fs::write(root.path().join(".git"), "gitdir: meta/wt\n").unwrap();
        assert_eq!(read_branch(root.path()).as_deref(), Some("rel"));
    }

    #[test]
    fn garbage_returns_none() {
        let dir = repo_with_head("hello");
        assert_eq!(read_branch(dir.path()), None);
    }
}
