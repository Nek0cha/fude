//! このマシンのフォルダの ProjectFs。

use std::ffi::OsString;
use std::fs;
use std::io;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fude_core::git_head::read_branch;
use ignore::WalkBuilder;

use crate::{DirEntry, FileMeta, IGNORED_FILES_MAX, ProjectFs, RelPath, TrashTicket, WALK_MAX, WalkedFiles, ignored};

/// 一時ファイルの名前を、同じプロセスの中の同時の書き込みで重ねないための通し番号。
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub struct LocalFs {
    root: PathBuf,
}

/// walker が返すファイル（シンボリックリンクの先がファイルのものも）の、root からの相対パス。
fn walked_file(root: &Path, entry: &ignore::DirEntry) -> Option<RelPath> {
    if !entry.file_type().is_some_and(|t| t.is_file() || (t.is_symlink() && entry.path().is_file())) {
        return None;
    }
    let rel = entry.path().strip_prefix(root).ok()?;
    RelPath::new(&rel.to_string_lossy()).ok()
}

/// walk_files の中身。外していないものを WALK_MAX 件まで歩き、そのあと外したものだけを ignored_max 件まで
/// 集める（2 回歩くことで、外したものの山で外していないものが打ち切られないようにする）。
pub(crate) fn walk_with_cap(root: &Path, ignored_max: usize) -> io::Result<WalkedFiles> {
    if !fs::metadata(root)?.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotADirectory, "ルートがフォルダではない"));
    }
    let in_repo = ignored::in_git_repo(root);
    // git のリポジトリでなければ、隠す名前のフォルダは名前だけで外す。
    let tracked_walker = WalkBuilder::new(root)
        .hidden(false)
        .parents(true)
        .filter_entry(move |e| {
            e.file_name() != ".git"
                && !(e.file_type().is_some_and(|t| t.is_dir()) && ignored::hides_dir(&e.file_name().to_string_lossy(), false, in_repo))
        })
        .build();
    let mut tracked = Vec::new();
    for entry in tracked_walker.flatten() {
        if let Some(rel) = walked_file(root, &entry) {
            tracked.push(rel);
            if tracked.len() >= WALK_MAX {
                break;
            }
        }
    }
    let known: HashSet<&str> = tracked.iter().map(RelPath::as_str).collect();
    // 規則を効かせずに歩き、外していないものに無いファイルを集める。隠す名前のフォルダには入らない
    // （外していない同じ名前のフォルダの中身は、上で集めてある）。
    let all_walker = WalkBuilder::new(root)
        .standard_filters(false)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git" && !(e.file_type().is_some_and(|t| t.is_dir()) && ignored::is_hidden_dir(&e.file_name().to_string_lossy())))
        .build();
    let mut ignored_files = Vec::new();
    for entry in all_walker.flatten() {
        let Some(rel) = walked_file(root, &entry) else { continue };
        if known.contains(rel.as_str()) {
            continue;
        }
        if ignored_files.len() >= ignored_max {
            eprintln!("外したファイルが {ignored_max} 件に達したので、残りは一覧に入れない");
            break;
        }
        ignored_files.push(rel);
    }
    drop(known);
    tracked.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    ignored_files.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    Ok(WalkedFiles { tracked, ignored: ignored_files })
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
        // 外したものも出して印を付ける。外されずに残る名前は、上位のフォルダの規則も効かせて求める。
        let kept = ignored::kept_names(&dir)?;
        let dir_ignored = ignored::is_ignored_path(&self.root, rel);
        let in_repo = ignored::in_git_repo(&self.root);
        let mut entries = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            let is_dir = match entry.file_type() {
                Ok(t) if t.is_symlink() => entry.path().is_dir(),
                Ok(t) => t.is_dir(),
                Err(_) => false,
            };
            let is_ignored = dir_ignored || !kept.contains(&name);
            let name = name.to_string_lossy().into_owned();
            if is_dir && ignored::hides_dir(&name, is_ignored, in_repo) {
                continue;
            }
            entries.push(DirEntry { name, is_dir, ignored: is_ignored });
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

    fn walk_files(&self) -> io::Result<WalkedFiles> {
        walk_with_cap(&self.root, IGNORED_FILES_MAX)
    }

    fn create_file(&self, rel: &RelPath) -> io::Result<()> {
        let path = self.non_root(rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::OpenOptions::new().write(true).create_new(true).open(path).map(|_| ())
    }

    fn create_dir(&self, rel: &RelPath) -> io::Result<()> {
        let path = self.non_root(rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // 最後の段は create_dir で作り、既にあれば AlreadyExists にする。
        fs::create_dir(path)
    }

    fn rename(&self, from: &RelPath, to: &RelPath) -> io::Result<()> {
        let (from, to) = (self.non_root(from)?, self.non_root(to)?);
        // 大文字小文字だけを変える名前の変更（Windows では同じファイル）は通す。
        let same_file = match (fs::canonicalize(&from), fs::canonicalize(&to)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if fs::symlink_metadata(&to).is_ok() && !same_file {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("すでにある: {}", to.display())));
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(from, to)
    }

    fn remove(&self, rel: &RelPath) -> io::Result<()> {
        let path = self.non_root(rel)?;
        let meta = fs::symlink_metadata(&path)?;
        if meta.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
    }

    fn trash(&self, rel: &RelPath) -> io::Result<TrashTicket> {
        crate::trash::move_to_trash(&self.non_root(rel)?)
    }

    fn restore(&self, ticket: &TrashTicket, rel: &RelPath) -> io::Result<()> {
        crate::trash::restore_from_trash(ticket, &self.non_root(rel)?)
    }

    fn metadata(&self, rel: &RelPath) -> io::Result<FileMeta> {
        let meta = fs::metadata(rel.to_path(&self.root))?;
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        Ok(FileMeta { is_dir: meta.is_dir(), len: meta.len(), modified_ms })
    }

    fn git_status(&self) -> io::Result<Option<crate::git::GitStatus>> {
        crate::git::git_status(&self.root)
    }

    fn git_diff(&self, rel: &RelPath) -> io::Result<String> {
        crate::git::git_diff(&self.root, rel)
    }

    fn plan_files(&self, known: Option<&crate::plans::PlanStamp>) -> io::Result<crate::plans::PlanFetch> {
        crate::plans::plan_files(&self.root, known)
    }

    fn search(&self, query: &crate::search::SearchQuery) -> io::Result<crate::search::SearchResults> {
        crate::search::search_dir(&self.root, query)
    }
}

impl LocalFs {
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// ルート自身でなければ、実際のパス。
    fn non_root(&self, rel: &RelPath) -> io::Result<PathBuf> {
        if rel.is_root() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "ルート自身は対象にできない"));
        }
        Ok(rel.to_path(&self.root))
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
        // 外した a.log は印を付けて出す。外された target（隠す名前）と .git は出さない。
        assert_eq!(names(&entries), vec![("src", true), (".env", false), (".gitignore", false), ("README.md", false), ("a.log", false)]);
        assert!(entries.iter().all(|e| e.ignored == (e.name == "a.log")));
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
        let got: Vec<(&str, bool)> = entries.iter().map(|e| (e.name.as_str(), e.ignored)).collect();
        assert_eq!(got, vec![("x.tmp", true), ("y.rs", false)]);
    }

    #[test]
    fn list_dir_of_missing_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let err = LocalFs::new(dir.path().to_path_buf()).list_dir(&rel("none")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    /// git のリポジトリの形のフォルダを作り、files を書く（"/" で終わるものはフォルダ）。
    fn repo(git: bool, files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if git {
            fs::create_dir_all(dir.path().join(".git")).unwrap();
        }
        for (path, body) in files {
            let p = dir.path().join(path);
            if path.ends_with('/') {
                fs::create_dir_all(&p).unwrap();
            } else {
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(&p, body).unwrap();
            }
        }
        dir
    }

    fn marks(fs_: &LocalFs, at: &str) -> Vec<(String, bool)> {
        let rel = if at.is_empty() { RelPath::root() } else { rel(at) };
        fs_.list_dir(&rel).unwrap().into_iter().map(|e| (e.name, e.ignored)).collect()
    }

    fn pair(name: &str, ignored: bool) -> (String, bool) {
        (name.to_string(), ignored)
    }

    #[test]
    fn list_dir_marks_ignored_entries() {
        let dir = repo(true, &[(".gitignore", "docs/\n*.log\n"), ("docs/plan.md", ""), ("src/a.rs", ""), ("a.log", ""), ("README.md", "")]);
        let got = marks(&LocalFs::new(dir.path().to_path_buf()), "");
        assert_eq!(got, vec![pair("docs", true), pair("src", false), pair(".gitignore", false), pair("README.md", false), pair("a.log", true)]);
    }

    #[test]
    fn list_dir_hides_heavy_ignored_dirs() {
        let dir = repo(true, &[(".gitignore", "node_modules/\ntarget/\n"), ("node_modules/x.js", ""), ("target/y", ""), ("src/a.rs", "")]);
        let got = marks(&LocalFs::new(dir.path().to_path_buf()), "");
        assert_eq!(got, vec![pair("src", false), pair(".gitignore", false)]);
    }

    #[test]
    fn list_dir_keeps_tracked_vendor() {
        let dir = repo(true, &[("vendor/lib.go", "")]);
        assert_eq!(marks(&LocalFs::new(dir.path().to_path_buf()), ""), vec![pair("vendor", false)]);
    }

    #[test]
    fn list_dir_without_git_hides_by_name() {
        let dir = repo(false, &[("node_modules/x.js", ""), ("src/a.rs", "")]);
        assert_eq!(marks(&LocalFs::new(dir.path().to_path_buf()), ""), vec![pair("src", false)]);
    }

    #[test]
    fn list_dir_inside_ignored_dir_marks_children() {
        let dir = repo(true, &[(".gitignore", "docs/\n"), ("docs/a.md", ""), ("docs/sub/b.md", ""), ("docs/node_modules/x.js", "")]);
        let got = marks(&LocalFs::new(dir.path().to_path_buf()), "docs");
        assert_eq!(got, vec![pair("sub", true), pair("a.md", true)]);
    }

    #[test]
    fn negated_file_inside_ignored_dir_is_tracked() {
        let dir = repo(true, &[(".gitignore", "docs/*\n!docs/keep.md\n"), ("docs/keep.md", ""), ("docs/other.md", "")]);
        let got = marks(&LocalFs::new(dir.path().to_path_buf()), "docs");
        assert_eq!(got, vec![pair("keep.md", false), pair("other.md", true)]);
    }

    #[test]
    fn dot_ignore_counts_as_ignored() {
        let dir = repo(true, &[(".ignore", "notes/\n"), ("notes/a.md", ""), ("src/a.rs", "")]);
        let got = marks(&LocalFs::new(dir.path().to_path_buf()), "");
        assert_eq!(got, vec![pair("notes", true), pair("src", false), pair(".ignore", false)]);
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

    #[test]
    fn walk_files_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git").join("HEAD"), "x").unwrap();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target").join("x"), "").unwrap();
        fs::create_dir_all(root.join("src").join("ui")).unwrap();
        fs::write(root.join("src").join("ui").join("b.rs"), "").unwrap();
        fs::write(root.join("src").join("main.rs"), "").unwrap();
        fs::write(root.join(".env"), "").unwrap();
        let files = LocalFs::new(root.to_path_buf()).walk_files().unwrap();
        let names: Vec<&str> = files.tracked.iter().map(|f| f.as_str()).collect();
        assert_eq!(names, vec![".env", ".gitignore", "src/main.rs", "src/ui/b.rs"]);
        // target は隠す名前なので、外したものの一覧にも入らない。
        assert!(files.ignored.is_empty());
    }

    fn strs(files: &[RelPath]) -> Vec<&str> {
        files.iter().map(|f| f.as_str()).collect()
    }

    #[test]
    fn walk_files_splits_tracked_and_ignored() {
        let dir = repo(true, &[(".gitignore", "docs/\n"), ("docs/plan.md", ""), ("src/a.rs", ""), ("README.md", "")]);
        let files = LocalFs::new(dir.path().to_path_buf()).walk_files().unwrap();
        assert_eq!(strs(&files.tracked), vec![".gitignore", "README.md", "src/a.rs"]);
        assert_eq!(strs(&files.ignored), vec!["docs/plan.md"]);
    }

    #[test]
    fn walk_files_skips_hidden_dirs() {
        let dir = repo(true, &[(".gitignore", "node_modules/\n"), ("node_modules/x.js", ""), ("a.rs", "")]);
        let files = LocalFs::new(dir.path().to_path_buf()).walk_files().unwrap();
        assert_eq!(strs(&files.tracked), vec![".gitignore", "a.rs"]);
        assert!(files.ignored.is_empty());
    }

    #[test]
    fn walk_files_without_git_skips_hidden_by_name() {
        let dir = repo(false, &[("node_modules/x.js", ""), ("a.rs", "")]);
        let files = LocalFs::new(dir.path().to_path_buf()).walk_files().unwrap();
        assert_eq!(strs(&files.tracked), vec!["a.rs"]);
    }

    #[test]
    fn ignored_cap_keeps_tracked() {
        let mut spec = vec![(".gitignore", "logs/\n"), ("a.rs", ""), ("b.rs", "")];
        let logs = ["logs/1", "logs/2", "logs/3", "logs/4", "logs/5"];
        spec.extend(logs.iter().map(|p| (*p, "")));
        let dir = repo(true, &spec);
        let files = walk_with_cap(dir.path(), 2).unwrap();
        assert_eq!(strs(&files.tracked), vec![".gitignore", "a.rs", "b.rs"]);
        assert_eq!(files.ignored.len(), 2);
    }

    #[test]
    fn create_rename_remove() {
        let dir = tempfile::tempdir().unwrap();
        let fs_ = LocalFs::new(dir.path().to_path_buf());
        fs_.create_file(&rel("a.txt")).unwrap();
        assert_eq!(fs_.create_file(&rel("a.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        fs_.create_file(&rel("b.txt")).unwrap();
        assert_eq!(fs_.rename(&rel("a.txt"), &rel("b.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        fs_.rename(&rel("a.txt"), &rel("c.txt")).unwrap();
        assert!(dir.path().join("c.txt").is_file() && !dir.path().join("a.txt").exists());
        fs_.create_dir(&rel("d/e")).unwrap();
        fs_.write(&rel("d/e/f.txt"), b"x").unwrap();
        let meta = fs_.metadata(&rel("d/e/f.txt")).unwrap();
        assert!(!meta.is_dir && meta.len == 1 && meta.modified_ms > 0);
        assert!(fs_.metadata(&rel("d")).unwrap().is_dir);
        fs_.remove(&rel("d")).unwrap();
        assert!(!dir.path().join("d").exists());
        fs_.remove(&rel("c.txt")).unwrap();
        assert!(fs_.remove(&RelPath::root()).is_err());
        assert_eq!(fs_.metadata(&rel("none")).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    /// 既にあるフォルダの名前で作ると AlreadyExists（取り消しで既にあったものを動かさないため）。
    #[test]
    fn create_dir_existing_is_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let fs_ = LocalFs::new(dir.path().to_path_buf());
        fs_.create_dir(&rel("a/b")).unwrap();
        assert_eq!(fs_.create_dir(&rel("a/b")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs_.create_dir(&rel("a")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    }

    // 以下は本物のゴミ箱を使う。作ったものは試験の中で戻して片付ける。

    #[test]
    fn trash_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let fs_ = LocalFs::new(dir.path().to_path_buf());
        fs::write(dir.path().join("t.txt"), "one").unwrap();
        let ticket = fs_.trash(&rel("t.txt")).unwrap();
        assert!(!dir.path().join("t.txt").exists());
        fs_.restore(&ticket, &rel("t.txt")).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("t.txt")).unwrap(), "one");
    }

    #[test]
    fn same_name_twice_restores_each() {
        let dir = tempfile::tempdir().unwrap();
        let fs_ = LocalFs::new(dir.path().to_path_buf());
        let p = dir.path().join("s.txt");
        fs::write(&p, "first").unwrap();
        let t1 = fs_.trash(&rel("s.txt")).unwrap();
        fs::write(&p, "second").unwrap();
        let t2 = fs_.trash(&rel("s.txt")).unwrap();
        assert_ne!(t1, t2);
        fs_.restore(&t2, &rel("s.txt")).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "second");
        fs::remove_file(&p).unwrap();
        fs_.restore(&t1, &rel("s.txt")).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "first");
    }

    #[test]
    fn restore_onto_existing_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let fs_ = LocalFs::new(dir.path().to_path_buf());
        let p = dir.path().join("c.txt");
        fs::write(&p, "old").unwrap();
        let t = fs_.trash(&rel("c.txt")).unwrap();
        fs::write(&p, "new").unwrap();
        assert_eq!(fs_.restore(&t, &rel("c.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&p).unwrap(), "new");
        fs::remove_file(&p).unwrap();
        fs_.restore(&t, &rel("c.txt")).unwrap();
    }

    #[test]
    fn trash_folder_and_root_refused() {
        let dir = tempfile::tempdir().unwrap();
        let fs_ = LocalFs::new(dir.path().to_path_buf());
        fs::create_dir_all(dir.path().join("d/e")).unwrap();
        fs::write(dir.path().join("d/e/f.txt"), "x").unwrap();
        let t = fs_.trash(&rel("d")).unwrap();
        assert!(!dir.path().join("d").exists());
        fs_.restore(&t, &rel("d")).unwrap();
        assert!(dir.path().join("d/e/f.txt").exists());
        assert!(fs_.trash(&RelPath::root()).is_err());
        assert_eq!(fs_.trash(&rel("none")).unwrap_err().kind(), io::ErrorKind::NotFound);
        let missing = crate::TrashTicket { id: "no-such".into() };
        assert_eq!(fs_.restore(&missing, &rel("z")).unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}
