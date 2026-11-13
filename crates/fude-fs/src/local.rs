//! このマシンのフォルダの ProjectFs。

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use fude_core::git_head::read_branch;
use ignore::WalkBuilder;

use crate::{DirEntry, ProjectFs, RelPath};

/// 一時ファイルの名前を、同じプロセスの中の同時の書き込みで重ねないための通し番号。
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub struct LocalFs {
    root: PathBuf,
}

impl LocalFs {
    pub fn new(root: PathBuf) -> LocalFs {
        LocalFs { root }
    }
}

impl ProjectFs for LocalFs {
    fn list_dir(&self, rel: &RelPath) -> io::Result<Vec<DirEntry>> {
        let dir = rel.to_path(&self.root);
        if !fs::metadata(&dir)?.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, format!("フォルダではない: {}", rel.as_str())));
        }
        // parents(true) で、上位のディレクトリの .gitignore も効かせる。
        let walker = WalkBuilder::new(&dir)
            .max_depth(Some(1))
            .hidden(false)
            .parents(true)
            .filter_entry(|e| e.file_name() != ".git")
            .build();
        let mut entries = Vec::new();
        for entry in walker {
            let Ok(entry) = entry else { continue };
            if entry.depth() == 0 {
                continue;
            }
            let is_dir = match entry.file_type() {
                Some(t) if t.is_symlink() => entry.path().is_dir(),
                Some(t) => t.is_dir(),
                None => false,
            };
            entries.push(DirEntry { name: entry.file_name().to_string_lossy().into_owned(), is_dir });
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        Ok(entries)
    }

    fn read(&self, rel: &RelPath) -> io::Result<Vec<u8>> {
        fs::read(rel.to_path(&self.root))
    }

    fn write(&self, rel: &RelPath, data: &[u8]) -> io::Result<()> {
        if rel.is_root() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "ルート自身には書けない"));
        }
        let path = rel.to_path(&self.root);
        let Some(parent) = path.parent() else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "書き込み先が無い"));
        };
        fs::create_dir_all(parent)?;
        // シンボリックリンクなら、リンクを残して先の実体を書き換える。
        let target = match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => fs::canonicalize(&path)?,
            _ => path.clone(),
        };
        let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "書き込み先が無い"));
        };
        let mut tmp_name = OsString::from(".");
        tmp_name.push(name);
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        tmp_name.push(format!(".fude-tmp-{}-{seq}", std::process::id()));
        let tmp = dir.join(tmp_name);
        let result = fs::write(&tmp, data)
            .and_then(|()| match fs::metadata(&target) {
                // 元のファイルの権限（実行権限など）を引き継ぐ。
                Ok(m) => fs::set_permissions(&tmp, m.permissions()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            })
            .and_then(|()| fs::rename(&tmp, &target));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    fn branch(&self) -> io::Result<Option<String>> {
        Ok(read_branch(&self.root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn rel(p: &str) -> RelPath {
        RelPath::new(p).unwrap()
    }

    fn names(entries: &[DirEntry]) -> Vec<(&str, bool)> {
        entries.iter().map(|e| (e.name.as_str(), e.is_dir)).collect()
    }

    #[test]
    fn list_dir_respects_gitignore_and_hides_dot_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target").join("x"), "").unwrap();
        fs::write(root.join("a.log"), "").unwrap();
        fs::write(root.join(".env"), "").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("main.rs"), "").unwrap();
        fs::write(root.join("README.md"), "").unwrap();
        let fs_ = LocalFs::new(root.to_path_buf());
        let entries = fs_.list_dir(&RelPath::root()).unwrap();
        assert_eq!(names(&entries), vec![("src", true), (".env", false), (".gitignore", false), ("README.md", false)]);
    }

    #[test]
    fn list_dir_applies_parent_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("x.tmp"), "").unwrap();
        fs::write(root.join("src").join("y.rs"), "").unwrap();
        let entries = LocalFs::new(root.to_path_buf()).list_dir(&rel("src")).unwrap();
        assert_eq!(names(&entries), vec![("y.rs", false)]);
    }

    #[test]
    fn list_dir_of_missing_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let err = LocalFs::new(dir.path().to_path_buf()).list_dir(&rel("none")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn read_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let fs_ = LocalFs::new(dir.path().to_path_buf());
        let data = [0u8, 10, 13, 255];
        fs_.write(&rel("dir/f.bin"), &data).unwrap();
        assert_eq!(fs_.read(&rel("dir/f.bin")).unwrap(), data);
        fs_.write(&rel("dir/f.bin"), b"second").unwrap();
        assert_eq!(fs_.read(&rel("dir/f.bin")).unwrap(), b"second");
        let left: Vec<_> = fs::read_dir(dir.path().join("dir")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("f.bin")]);
        assert_eq!(fs_.read(&rel("none")).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn write_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.sh");
        fs::write(&path, "#!/bin/sh").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        LocalFs::new(dir.path().to_path_buf()).write(&rel("run.sh"), b"echo hi").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn write_through_symlink_keeps_link() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "old").unwrap();
        std::os::unix::fs::symlink("AGENTS.md", dir.path().join("CLAUDE.md")).unwrap();
        LocalFs::new(dir.path().to_path_buf()).write(&rel("CLAUDE.md"), b"new").unwrap();
        assert!(fs::symlink_metadata(dir.path().join("CLAUDE.md")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(dir.path().join("AGENTS.md")).unwrap(), b"new");
    }

    #[test]
    fn branch_reads_head() {
        let dir = tempfile::tempdir().unwrap();
        let fs_ = LocalFs::new(dir.path().to_path_buf());
        assert_eq!(fs_.branch().unwrap(), None);
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(fs_.branch().unwrap(), Some("main".to_string()));
    }
}
