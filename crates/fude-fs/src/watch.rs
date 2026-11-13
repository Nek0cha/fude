//! プロジェクトのファイルの変更を監視する。
//!
//! 監視する範囲: ignore の歩き方で得た「無視されないディレクトリ」の集合を持ち、変わったものの親がその集合に
//! 無ければ捨てる（node_modules や target を読み込まないため。設計書 §2 のアンチゴール）。Linux はその集合の
//! 各ディレクトリを再帰なしで監視し（inotify の監視数を無視するディレクトリで使い切らないため）、ほかの OS は
//! ルートを再帰で監視して集合で絞る。`.git` は HEAD、index、refs の変化だけを `git` の印として知らせる。

use std::collections::{BTreeSet, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};

use crate::RelPath;

/// 変更をまとめる時間。最初の変更から数える。
const BATCH: Duration = Duration::from_millis(200);

/// まとめた変更。`fude watch` はこれを JSON 1 行ずつ書く。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WatchBatch {
    /// 変わったファイルやフォルダ（消えたものも含む）。名前順。
    pub changed: Vec<RelPath>,
    /// .git の HEAD、index、refs が変わった（ブランチや状態を読み直す）。
    pub git: bool,
}

type OnBatch = Box<dyn Fn(WatchBatch) + Send>;

/// このマシンのフォルダの監視。捨てると止まる。
pub struct LocalWatcher {
    _watcher: RecommendedWatcher,
}

/// Linux は各ディレクトリを再帰なしで監視する。
const PER_DIR: bool = cfg!(target_os = "linux");

impl LocalWatcher {
    pub fn start(root: PathBuf, on_batch: OnBatch) -> io::Result<LocalWatcher> {
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(tx).map_err(io::Error::other)?;
        let dirs = walk_dirs(&root);
        let git_dir = root.join(".git");
        if PER_DIR {
            for dir in &dirs {
                let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
            }
            for dir in [git_dir.clone(), git_dir.join("refs").join("heads")] {
                let _ = watcher.watch(&dir, RecursiveMode::NonRecursive);
            }
        } else {
            watcher.watch(&root, RecursiveMode::Recursive).map_err(io::Error::other)?;
        }
        // 新しいディレクトリの監視は、ワーカーから Watcher を触れないため、別の Watcher に足していく
        // （Linux だけ。ほかの OS は再帰の監視で届く）。
        let extra = if PER_DIR {
            let (etx, erx) = mpsc::channel::<notify::Result<notify::Event>>();
            let extra = notify::recommended_watcher(etx).map_err(io::Error::other)?;
            Some((extra, erx))
        } else {
            None
        };
        std::thread::Builder::new()
            .name("fude-watch".into())
            .spawn(move || worker(root, dirs, rx, extra, on_batch))?;
        Ok(LocalWatcher { _watcher: watcher })
    }
}

/// root の下の、無視されないディレクトリ（start 自身を含む）。.git は含めない。
fn walk_dirs(start: &Path) -> HashSet<PathBuf> {
    WalkBuilder::new(start)
        .hidden(false)
        .parents(true)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_dir()))
        .map(|e| e.into_path())
        .collect()
}

/// dir が、その親のディレクトリの一覧（gitignore を適用したもの）に出てくるか。
fn not_ignored(dir: &Path) -> bool {
    let Some(parent) = dir.parent() else { return false };
    WalkBuilder::new(parent)
        .max_depth(Some(1))
        .hidden(false)
        .parents(true)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .any(|e| e.path() == dir)
}

/// .git の下で、状態の読み直しが要るもの。
fn is_git_signal(rel: &Path) -> bool {
    let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
    if parts.next().as_deref() != Some(".git") {
        return false;
    }
    matches!(parts.next().as_deref(), Some("HEAD" | "index" | "refs" | "packed-refs"))
}

fn worker(
    root: PathBuf,
    mut dirs: HashSet<PathBuf>,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    mut extra: Option<(RecommendedWatcher, mpsc::Receiver<notify::Result<notify::Event>>)>,
    on_batch: OnBatch,
) {
    let mut changed: BTreeSet<String> = BTreeSet::new();
    let mut git = false;
    let mut first: Option<Instant> = None;
    loop {
        let wait = first.map_or(Duration::from_millis(100), |t| BATCH.saturating_sub(t.elapsed()));
        let mut events = Vec::new();
        match rx.recv_timeout(wait.max(Duration::from_millis(1))) {
            Ok(ev) => events.push(ev),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if let Some((_, erx)) = &extra {
            events.extend(erx.try_iter());
        }
        for ev in events.into_iter().filter_map(Result::ok) {
            for path in ev.paths {
                let Ok(rel) = path.strip_prefix(&root) else { continue };
                if rel.starts_with(".git") {
                    if is_git_signal(rel) {
                        git = true;
                        first.get_or_insert_with(Instant::now);
                    }
                    continue;
                }
                let Some(parent) = path.parent() else { continue };
                if !dirs.contains(parent) {
                    continue;
                }
                if path.is_dir() && !dirs.contains(&path) && not_ignored(&path) {
                    let added = walk_dirs(&path);
                    if let Some((w, _)) = extra.as_mut() {
                        for d in &added {
                            let _ = w.watch(d, RecursiveMode::NonRecursive);
                        }
                    }
                    // 監視を始める前にできた中身も知らせる。
                    for d in &added {
                        if let Ok(read) = std::fs::read_dir(d) {
                            for e in read.flatten() {
                                if let Ok(r) = e.path().strip_prefix(&root) {
                                    changed.insert(r.to_string_lossy().replace('\\', "/"));
                                }
                            }
                        }
                    }
                    dirs.extend(added);
                }
                changed.insert(rel.to_string_lossy().replace('\\', "/"));
                first.get_or_insert_with(Instant::now);
            }
        }
        if first.is_some_and(|t| t.elapsed() >= BATCH) {
            let batch = WatchBatch {
                changed: std::mem::take(&mut changed).iter().filter_map(|p| RelPath::new(p).ok()).collect(),
                git: std::mem::take(&mut git),
            };
            first = None;
            on_batch(batch);
        }
    }
}

/// 起動し直すまでの待ち時間。1s、2s、4s …最大 30s。
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64.checked_shl(attempt).unwrap_or(u64::MAX).min(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// 監視を始め、届いたまとまりを受け取る口を返す。監視の準備が整うまで少し待つ。
    fn start(root: &std::path::Path) -> (LocalWatcher, mpsc::Receiver<WatchBatch>) {
        let (tx, rx) = mpsc::channel();
        let watcher = LocalWatcher::start(root.to_path_buf(), Box::new(move |b| {
            let _ = tx.send(b);
        }))
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        (watcher, rx)
    }

    /// deadline までに届いたまとまりを、条件を満たすまで集める。
    fn wait_for(rx: &mpsc::Receiver<WatchBatch>, within: Duration, done: impl Fn(&[WatchBatch]) -> bool) -> Vec<WatchBatch> {
        let deadline = Instant::now() + within;
        let mut got = Vec::new();
        while Instant::now() < deadline && !done(&got) {
            if let Ok(b) = rx.recv_timeout(Duration::from_millis(50)) {
                got.push(b);
            }
        }
        got
    }

    fn changed(batches: &[WatchBatch]) -> Vec<String> {
        batches.iter().flat_map(|b| b.changed.iter().map(|p| p.as_str().to_string())).collect()
    }

    #[test]
    fn reports_created_file_within_two_seconds() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let (_w, rx) = start(dir.path());
        std::fs::write(dir.path().join("src").join("a.rs"), "x").unwrap();
        let got = wait_for(&rx, Duration::from_secs(2), |b| changed(b).contains(&"src/a.rs".to_string()));
        assert!(changed(&got).contains(&"src/a.rs".to_string()), "{got:?}");
    }

    #[test]
    fn ignored_dir_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        let (_w, rx) = start(dir.path());
        std::fs::write(dir.path().join("target").join("x"), "x").unwrap();
        std::fs::write(dir.path().join("b.rs"), "x").unwrap();
        let got = wait_for(&rx, Duration::from_secs(2), |b| changed(b).contains(&"b.rs".to_string()));
        let names = changed(&got);
        assert!(names.contains(&"b.rs".to_string()), "{got:?}");
        assert!(!names.iter().any(|n| n.starts_with("target")), "{got:?}");
    }

    #[test]
    fn new_dir_is_watched() {
        let dir = tempfile::tempdir().unwrap();
        let (_w, rx) = start(dir.path());
        std::fs::create_dir_all(dir.path().join("d")).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        std::fs::write(dir.path().join("d").join("e.rs"), "x").unwrap();
        let got = wait_for(&rx, Duration::from_secs(3), |b| changed(b).contains(&"d/e.rs".to_string()));
        assert!(changed(&got).contains(&"d/e.rs".to_string()), "{got:?}");
    }

    #[test]
    fn git_head_change_sets_flag() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git").join("refs").join("heads")).unwrap();
        std::fs::write(dir.path().join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let (_w, rx) = start(dir.path());
        std::fs::write(dir.path().join(".git").join("HEAD"), "ref: refs/heads/dev\n").unwrap();
        let got = wait_for(&rx, Duration::from_secs(2), |b| b.iter().any(|x| x.git));
        assert!(got.iter().any(|b| b.git), "{got:?}");
        assert!(!changed(&got).iter().any(|n| n.starts_with(".git")), "{got:?}");
    }

    #[test]
    fn batch_roundtrips_as_json_line() {
        let b = WatchBatch { changed: vec![RelPath::new("a/b c.rs").unwrap()], git: true };
        let line = serde_json::to_string(&b).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<WatchBatch>(&line).unwrap(), b);
    }

    #[test]
    fn wsl_watch_restarts_with_backoff() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(10), Duration::from_secs(30));
        assert_eq!(backoff(100), Duration::from_secs(30));
    }
}
