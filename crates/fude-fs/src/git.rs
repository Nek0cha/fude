//! Git の状態と差分を、git コマンドで読む（設計書 §5.7。表示だけを行う）。
//! ユーザーの git の設定（改行の変換、属性、フィルター）をそのまま反映するため、ライブラリではなくコマンドを使う。

use std::io;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::RelPath;

/// ツリーと変更の一覧に出す Git の記号（M U D A）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitMark {
    Modified,
    Untracked,
    Deleted,
    Added,
}

/// git status の結果。プロジェクトのルートからの相対パスで、名前順。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GitStatus {
    pub entries: Vec<(RelPath, GitMark)>,
}

impl GitStatus {
    pub fn mark(&self, rel: &RelPath) -> Option<GitMark> {
        self.entries.iter().find(|(p, _)| p == rel).map(|(_, m)| *m)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 変更された行の印。行番号は新しい側の 1 始まり。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineMark {
    Added(u32),
    Modified(u32),
    /// この行の直前で行が削除された。
    DeletedAbove(u32),
}

/// コンソールの窓を出さない。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// dir で git を動かし、成功したら標準出力を返す。git が見つからない、または失敗した（リポジトリでないなど）→ Ok(None)。
/// index.lock を取らないよう `--no-optional-locks` と GIT_OPTIONAL_LOCKS=0 を付ける（Claude Code が同時に git を使うため）。
pub fn run_git(dir: &Path, args: &[&str]) -> io::Result<Option<Vec<u8>>> {
    let mut cmd = Command::new("git");
    cmd.args(["--no-optional-locks", "-c", "core.quotepath=off"])
        .args(args)
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    match cmd.output() {
        Ok(out) if out.status.success() => Ok(Some(out.stdout)),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// `git status --porcelain=v1 -z` の出力を読む。prefix（`git rev-parse --show-prefix`）で始まらない項目は捨て、
/// 残りは prefix を外す。名前順に並べる。
pub fn parse_porcelain(bytes: &[u8], prefix: &str) -> Vec<(RelPath, GitMark)> {
    let mut fields = bytes.split(|b| *b == 0);
    let mut entries = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let (x, y) = (field[0], field[1]);
        if matches!(x, b'R' | b'C') {
            // 続く欄は元の名前。
            fields.next();
        }
        let mark = match (x, y) {
            (b'?', b'?') => GitMark::Untracked,
            (b'!', b'!') => continue,
            (_, b'D') | (b'D', _) => GitMark::Deleted,
            (b'A', _) => GitMark::Added,
            _ => GitMark::Modified,
        };
        let path = String::from_utf8_lossy(&field[3..]);
        let Some(rest) = path.strip_prefix(prefix) else { continue };
        let Ok(rel) = RelPath::new(rest) else { continue };
        if rel.is_root() {
            continue;
        }
        entries.push((rel, mark));
    }
    entries.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    entries
}

/// unified diff から、新しい側の変更された行の印を作る。削除の直後の追加は Modified、追加だけなら Added、
/// 削除だけなら次の行に DeletedAbove。
pub fn line_marks(patch: &str) -> Vec<LineMark> {
    let mut marks = Vec::new();
    let mut line = 0u32;
    let mut in_hunk = false;
    let mut pending_del = 0u32;
    let flush = |marks: &mut Vec<LineMark>, pending: &mut u32, line: u32| {
        if *pending > 0 {
            marks.push(LineMark::DeletedAbove(line));
            *pending = 0;
        }
    };
    for text in patch.lines() {
        if let Some(header) = text.strip_prefix("@@ ") {
            flush(&mut marks, &mut pending_del, line);
            let Some(start) = header.split_whitespace().find_map(|p| p.strip_prefix('+')) else {
                in_hunk = false;
                continue;
            };
            line = start.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(0).max(1);
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        match text.as_bytes().first() {
            Some(b'+') => {
                if pending_del > 0 {
                    pending_del -= 1;
                    marks.push(LineMark::Modified(line));
                } else {
                    marks.push(LineMark::Added(line));
                }
                line += 1;
            }
            Some(b'-') => pending_del += 1,
            Some(b'\\') => {}
            _ => {
                flush(&mut marks, &mut pending_del, line);
                line += 1;
            }
        }
    }
    flush(&mut marks, &mut pending_del, line);
    marks
}

/// root で git が管理しているファイル（インデックスにあるもの。`git add -f` したものも含む）の、root からの
/// 相対パス（"/" 区切り）。under を渡すと、そのフォルダの下だけ。リポジトリでない、git が無い → Ok(None)。
pub fn index_files(root: &Path, under: Option<&RelPath>) -> io::Result<Option<Vec<String>>> {
    let mut args = vec!["ls-files", "-z", "--cached", "--"];
    let dir = under.filter(|u| !u.is_root()).map(|u| u.as_str().to_string());
    if let Some(dir) = &dir {
        args.push(dir);
    }
    let Some(out) = run_git(root, &args)? else { return Ok(None) };
    Ok(Some(out.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect()))
}

/// root の Git の状態。リポジトリでない、git が無い → Ok(None)。
pub fn git_status(root: &Path) -> io::Result<Option<GitStatus>> {
    let Some(prefix) = run_git(root, &["rev-parse", "--show-prefix"])? else { return Ok(None) };
    let prefix = String::from_utf8_lossy(&prefix).trim_end_matches(['\r', '\n']).to_string();
    let Some(out) = run_git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])? else {
        return Ok(None);
    };
    Ok(Some(GitStatus { entries: parse_porcelain(&out, &prefix) }))
}

/// rel の作業ツリーと HEAD の差分（unified diff）。HEAD が無い新しいリポジトリでは、インデックスとの差分。
/// 変更が無ければ空の文字列。
pub fn git_diff(root: &Path, rel: &RelPath) -> io::Result<String> {
    let has_head = run_git(root, &["rev-parse", "--verify", "-q", "HEAD"])?.is_some();
    let base = if has_head { "HEAD" } else { "--cached" };
    let args = ["diff", "--no-color", "--no-ext-diff", base, "--", rel.as_str()];
    match run_git(root, &args)? {
        Some(out) => Ok(String::from_utf8_lossy(&out).into_owned()),
        None => Err(io::Error::other(format!("git diff に失敗した: {}", rel.as_str()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn names(entries: &[(RelPath, GitMark)]) -> Vec<(&str, GitMark)> {
        entries.iter().map(|(p, m)| (p.as_str(), *m)).collect()
    }

    #[test]
    fn porcelain_parses_japanese_and_spaces() {
        let mut out = b" M src/a b.rs\0?? ".to_vec();
        out.extend("作業.md".as_bytes());
        out.extend(b"\0R  new.rs\0old.rs\0D  gone.rs\0A  added.rs\0 D wt-gone.rs\0MM both.rs\0");
        assert_eq!(
            names(&parse_porcelain(&out, "")),
            vec![
                ("added.rs", GitMark::Added),
                ("both.rs", GitMark::Modified),
                ("gone.rs", GitMark::Deleted),
                ("new.rs", GitMark::Modified),
                ("src/a b.rs", GitMark::Modified),
                ("wt-gone.rs", GitMark::Deleted),
                ("作業.md", GitMark::Untracked),
            ]
        );
    }

    #[test]
    fn status_in_subdirectory_strips_prefix() {
        let got = parse_porcelain(b" M sub/a.rs\0 M other/b.rs\0R  sub/n.rs\0other/o.rs\0", "sub/");
        assert_eq!(names(&got), vec![("a.rs", GitMark::Modified), ("n.rs", GitMark::Modified)]);
    }

    #[test]
    fn status_mark_and_len() {
        let status = GitStatus { entries: parse_porcelain(b" M a.rs\0?? b.rs\0", "") };
        assert_eq!(status.len(), 2);
        assert_eq!(status.mark(&RelPath::new("b.rs").unwrap()), Some(GitMark::Untracked));
        assert_eq!(status.mark(&RelPath::new("c.rs").unwrap()), None);
    }

    #[test]
    fn line_marks_from_patch() {
        let patch = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,3 +1,4 @@\n a\n-b\n+B\n+c\n d\n@@ -10,2 +11,1 @@\n x\n-y\n";
        assert_eq!(line_marks(patch), vec![LineMark::Modified(2), LineMark::Added(3), LineMark::DeletedAbove(12)]);
    }

    #[test]
    fn line_marks_pure_addition_and_no_newline_marker() {
        let patch = "@@ -0,0 +1,2 @@\n+a\n+b\n\\ No newline at end of file\n";
        assert_eq!(line_marks(patch), vec![LineMark::Added(1), LineMark::Added(2)]);
    }

    /// git が無い環境では何もしない。
    fn git(dir: &Path, args: &[&str]) -> bool {
        Command::new("git").args(args).current_dir(dir).output().is_ok_and(|o| o.status.success())
    }

    fn repo() -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().unwrap();
        if !git(dir.path(), &["init", "-q", "-b", "main"]) {
            return None;
        }
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        git(dir.path(), &["config", "core.autocrlf", "false"]);
        Some(dir)
    }

    #[test]
    fn status_and_diff_with_real_git() {
        let Some(dir) = repo() else { return };
        let root = dir.path();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("a.txt"), "1\n2\n3\n").unwrap();
        assert!(git(root, &["add", "."]) && git(root, &["commit", "-qm", "init"]));
        std::fs::write(root.join("sub").join("a.txt"), "1\nX\n3\n").unwrap();
        std::fs::write(root.join("sub").join("new.txt"), "n\n").unwrap();

        let status = git_status(root).unwrap().expect("リポジトリである");
        assert_eq!(names(&status.entries), vec![("sub/a.txt", GitMark::Modified), ("sub/new.txt", GitMark::Untracked)]);
        let patch = git_diff(root, &RelPath::new("sub/a.txt").unwrap()).unwrap();
        assert!(patch.contains("-2\n+X\n"), "{patch}");
        assert_eq!(line_marks(&patch), vec![LineMark::Modified(2)]);

        // サブディレクトリをプロジェクトにしたとき。
        let sub = root.join("sub");
        let status = git_status(&sub).unwrap().unwrap();
        assert_eq!(names(&status.entries), vec![("a.txt", GitMark::Modified), ("new.txt", GitMark::Untracked)]);
        assert!(git_diff(&sub, &RelPath::new("a.txt").unwrap()).unwrap().contains("+X"));
    }

    #[test]
    fn diff_without_head_shows_added_lines() {
        let Some(dir) = repo() else { return };
        std::fs::write(dir.path().join("a.txt"), "1\n").unwrap();
        assert!(git(dir.path(), &["add", "a.txt"]));
        let status = git_status(dir.path()).unwrap().unwrap();
        assert_eq!(names(&status.entries), vec![("a.txt", GitMark::Added)]);
        assert!(git_diff(dir.path(), &RelPath::new("a.txt").unwrap()).unwrap().contains("+1"));
    }

    #[test]
    fn status_without_git_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(git_status(dir.path()).unwrap().is_none());
    }

    #[test]
    fn status_does_not_take_index_lock() {
        let Some(dir) = repo() else { return };
        std::fs::write(dir.path().join("a.txt"), "1\n").unwrap();
        assert!(git(dir.path(), &["add", "."]) && git(dir.path(), &["commit", "-qm", "init"]));
        std::fs::write(dir.path().join("a.txt"), "2\n").unwrap();
        let lock = dir.path().join(".git").join("index.lock");
        std::fs::write(&lock, "").unwrap();
        let status = git_status(dir.path()).unwrap().unwrap();
        assert_eq!(names(&status.entries), vec![("a.txt", GitMark::Modified)]);
        assert!(lock.exists(), "他のプロセスの lock を消さない");
    }

    #[test]
    fn local_fs_exposes_git() {
        let Some(dir) = repo() else { return };
        std::fs::write(dir.path().join("a.txt"), "1\n").unwrap();
        let fs = crate::LocalFs::new(dir.path().to_path_buf());
        use crate::ProjectFs;
        assert_eq!(fs.git_status().unwrap().unwrap().len(), 1);
    }
}
