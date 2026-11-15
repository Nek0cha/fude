//! プロジェクトのファイルの変更を監視する。
//!
//! 監視する範囲: 外していないディレクトリと、外したディレクトリ（IGNORED_DIRS_MAX 個まで。計画8d）の集合を
//! 持ち、変わったものの親がその集合に無ければ捨てる。外された重いフォルダ（ignored::HIDDEN_DIRS）は入れない
//! （node_modules や target を読み込まないため。設計書 §2 のアンチゴール）。Linux はその集合の各ディレクトリを
//! 再帰なしで監視し（inotify の監視数を使い切らないため）、ほかの OS はルートを再帰で監視して集合で絞る。
//! `.git` は HEAD、index、refs の変化だけを `git` の印として、外す規則の変化を `rules` の印として知らせる。

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
    /// 外す規則（.gitignore、.ignore、.rgignore、.git/info/exclude）が変わった（外したかの印を読み直す）。
    #[serde(default)]
    pub rules: bool,
}

/// 監視する、外したフォルダの数の上限（Linux の inotify の見張りを使い切らないため）。
pub const IGNORED_DIRS_MAX: usize = 2_000;

/// 監視するフォルダ。ignored は、そのうち外したフォルダ（上限を数えるため）。
struct WatchSet {
    dirs: HashSet<PathBuf>,
    ignored: HashSet<PathBuf>,
}

fn is_dir_entry(e: &ignore::DirEntry) -> bool {
    e.file_type().is_some_and(|t| t.is_dir())
}

/// start の下の、外したフォルダ（外された隠す名前のフォルダとその中、.git は除く）を、skip（外していない
/// フォルダ）に無いものから max 個まで。
/// 浅い順（幅優先）に集める。深い生成物より、浅い docs/superpowers などを先に見張るため。
/// シンボリックリンクのフォルダは辿らない。
fn ignored_dirs(start: &Path, skip: &HashSet<PathBuf>, max: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if max == 0 {
        return out;
    }
    if !skip.contains(start) {
        out.push(start.to_path_buf());
    }
    let mut queue = std::collections::VecDeque::from([start.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        let mut children: Vec<PathBuf> = read
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| e.file_name() != ".git")
            .map(|e| e.path())
            // 隠す名前のフォルダは、外していないもの（skip にある、コミットした vendor/ など）でなければ入らない。
            .filter(|p| !p.file_name().is_some_and(|n| crate::ignored::is_hidden_dir(&n.to_string_lossy())) || skip.contains(p))
            .collect();
        children.sort();
        for child in children {
            if out.len() >= max {
                eprintln!("監視する外したフォルダが {max} 個に達したので、残りは見張らない");
                return out;
            }
            if !skip.contains(&child) {
                out.push(child.clone());
            }
            queue.push_back(child);
        }
    }
    out
}

/// start の下で監視するフォルダ。外していないフォルダはすべて、外したフォルダは ignored_max 個まで。
fn watch_set(start: &Path, root: &Path, ignored_max: usize) -> WatchSet {
    let dirs = walk_dirs(start, crate::ignored::in_git_repo(root));
    let ignored: HashSet<PathBuf> = ignored_dirs(start, &dirs, ignored_max).into_iter().collect();
    WatchSet { dirs: dirs.union(&ignored).cloned().collect(), ignored }
}

/// rel（root からの相対パス）の親の段に、隠す名前のフォルダがあるか。
fn has_hidden_component(rel: &Path) -> bool {
    rel.parent().is_some_and(|p| p.components().any(|c| crate::ignored::is_hidden_dir(&c.as_os_str().to_string_lossy())))
}

impl WatchSet {
    /// 消えたフォルダとその中を集合から外す。
    fn forget(&mut self, dir: &Path) {
        self.dirs.retain(|d| !d.starts_with(dir));
        self.ignored.retain(|d| !d.starts_with(dir));
    }
}

/// watch_set のフォルダの集合（試験用の入口）。
#[cfg(test)]
fn watch_dirs(start: &Path, root: &Path, ignored_max: usize) -> HashSet<PathBuf> {
    watch_set(start, root, ignored_max).dirs
}

/// 外す規則のファイルか（root からの相対パス）。
fn is_rules_file(rel: &Path) -> bool {
    if rel == Path::new(".git").join("info").join("exclude") {
        return true;
    }
    if rel.starts_with(".git") {
        return false;
    }
    matches!(rel.file_name().and_then(|n| n.to_str()), Some(".gitignore" | ".ignore" | ".rgignore"))
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
        // 外したフォルダを集合に入れるのは Linux だけ（フォルダごとに見張るため）。ほかの OS は再帰で見張り、
        // 届いた変化を has_hidden_component で絞る。
        let set = watch_set(&root, &root, if PER_DIR { IGNORED_DIRS_MAX } else { 0 });
        let git_dir = root.join(".git");
        if PER_DIR {
            for dir in &set.dirs {
                let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
            }
            for dir in [git_dir.clone(), git_dir.join("refs").join("heads"), git_dir.join("info")] {
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
            .spawn(move || worker(root, set, rx, extra, on_batch))?;
        Ok(LocalWatcher { _watcher: watcher })
    }
}

/// start の下の、外していないディレクトリ（start 自身を含む）。.git は含めない。git のリポジトリでなければ、
/// 隠す名前のフォルダは名前だけで除く。
fn walk_dirs(start: &Path, in_repo: bool) -> HashSet<PathBuf> {
    WalkBuilder::new(start)
        .hidden(false)
        .parents(true)
        .filter_entry(move |e| {
            e.file_name() != ".git" && !(is_dir_entry(e) && crate::ignored::hides_dir(&e.file_name().to_string_lossy(), false, in_repo))
        })
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_dir()))
        .map(|e| e.into_path())
        .collect()
}

/// .git の下で、状態の読み直しが要るもの。
fn is_git_signal(rel: &Path) -> bool {
    let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
    if parts.next().as_deref() != Some(".git") {
        return false;
    }
    matches!(parts.next().as_deref(), Some("HEAD" | "index" | "refs" | "packed-refs"))
}

/// 中身や並びが変わりうる通知か。開いた、読んだだけの通知（Linux の inotify が出す）は捨てる。
/// git status がファイルを読むたびに知らせると、状態の読み直しが止まらなくなるため。
fn changes_content(ev: &notify::Event) -> bool {
    use notify::EventKind;
    use notify::event::{AccessKind, AccessMode};
    match ev.kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

fn worker(
    root: PathBuf,
    set: WatchSet,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    mut extra: Option<(RecommendedWatcher, mpsc::Receiver<notify::Result<notify::Event>>)>,
    on_batch: OnBatch,
) {
    let mut set = set;
    let in_repo = crate::ignored::in_git_repo(&root);
    let mut changed: BTreeSet<String> = BTreeSet::new();
    let mut git = false;
    let mut rules = false;
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
        for ev in events.into_iter().filter_map(Result::ok).filter(changes_content) {
            for path in ev.paths {
                let Ok(rel) = path.strip_prefix(&root) else { continue };
                if rel.starts_with(".git") {
                    if is_rules_file(rel) {
                        rules = true;
                        first.get_or_insert_with(Instant::now);
                    } else if is_git_signal(rel) {
                        git = true;
                        first.get_or_insert_with(Instant::now);
                    }
                    continue;
                }
                let Some(parent) = path.parent() else { continue };
                // Linux は見張っているフォルダの中だけ。ほかの OS はルートを再帰で見張っているので、隠す名前の
                // フォルダの下でなければ通す（外したフォルダの数で絞らない）。
                if !set.dirs.contains(parent) && (PER_DIR || has_hidden_component(rel)) {
                    continue;
                }
                if is_rules_file(rel) {
                    rules = true;
                }
                // 消えたフォルダは集合から外す（外したフォルダの枠を空ける）。
                if !path.exists() && set.dirs.contains(&path) {
                    set.forget(&path);
                }
                if path.is_dir() && !set.dirs.contains(&path) {
                    // 外したフォルダの中にできたフォルダも、外したものとして数える。
                    // 祖先まで見る（外したフォルダの中から歩くと規則が効かないため）。
                    let is_ignored = set.ignored.contains(parent)
                        || RelPath::new(&rel.to_string_lossy().replace('\\', "/")).is_ok_and(|r| crate::ignored::is_ignored_path(&root, &r));
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let left = IGNORED_DIRS_MAX.saturating_sub(set.ignored.len());
                    if crate::ignored::hides_dir(&name, is_ignored, in_repo) {
                        // 隠すフォルダは、できたことも知らせない（ツリーにも出ないため）。
                        continue;
                    }
                    let added: HashSet<PathBuf> = if !PER_DIR {
                        // 見張りは要らない。中身を知らせるためだけに足す（数えない）。
                        HashSet::from([path.clone()])
                    } else if is_ignored {
                        let found = ignored_dirs(&path, &set.dirs, left);
                        set.ignored.extend(found.iter().cloned());
                        found.into_iter().collect()
                    } else {
                        let sub = watch_set(&path, &root, left);
                        set.ignored.extend(sub.ignored);
                        sub.dirs
                    };
                    if let Some((w, _)) = extra.as_mut() {
                        for d in &added {
                            let _ = w.watch(d, RecursiveMode::NonRecursive);
                        }
                    }
                    // 監視を始める前にできた中身も知らせる。隠すフォルダは知らせない。
                    for d in &added {
                        let d_ignored = is_ignored || set.ignored.contains(d);
                        if let Ok(read) = std::fs::read_dir(d) {
                            for e in read.flatten() {
                                let child_name = e.file_name().to_string_lossy().into_owned();
                                if e.file_type().is_ok_and(|t| t.is_dir()) && crate::ignored::hides_dir(&child_name, d_ignored, in_repo) {
                                    continue;
                                }
                                if let Ok(r) = e.path().strip_prefix(&root) {
                                    changed.insert(r.to_string_lossy().replace('\\', "/"));
                                }
                            }
                        }
                    }
                    set.dirs.extend(added);
                }
                changed.insert(rel.to_string_lossy().replace('\\', "/"));
                first.get_or_insert_with(Instant::now);
            }
        }
        if first.is_some_and(|t| t.elapsed() >= BATCH) {
            if rules {
                // 外す規則が変わったら、監視するフォルダを作り直す（外さなくなったフォルダを見張り始める）。
                let fresh = watch_set(&root, &root, if PER_DIR { IGNORED_DIRS_MAX } else { 0 });
                if let Some((w, _)) = extra.as_mut() {
                    for d in fresh.dirs.difference(&set.dirs) {
                        let _ = w.watch(d, RecursiveMode::NonRecursive);
                    }
                }
                set = fresh;
            }
            let batch = WatchBatch {
                changed: std::mem::take(&mut changed).iter().filter_map(|p| RelPath::new(p).ok()).collect(),
                git: std::mem::take(&mut git),
                rules: std::mem::take(&mut rules),
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
    fn reading_files_is_not_reported() {
        // git status はファイルと .git/index を読む。読んだだけで知らせると、状態の読み直しが止まらなくなる。
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git").join("index"), "i").unwrap();
        std::fs::write(dir.path().join("a.rs"), "x").unwrap();
        let (_w, rx) = start(dir.path());
        for _ in 0..3 {
            let _ = std::fs::read(dir.path().join("a.rs")).unwrap();
            let _ = std::fs::read(dir.path().join(".git").join("index")).unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        let got = wait_for(&rx, Duration::from_secs(1), |b| !b.is_empty());
        assert!(got.is_empty(), "{got:?}");
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
    fn ignored_dir_is_reported() {
        // 外したフォルダ（隠す名前でないもの）の中の変化も知らせる（計画8d）。
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "docs/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        let (_w, rx) = start(dir.path());
        std::fs::write(dir.path().join("docs").join("plan.md"), "x").unwrap();
        let got = wait_for(&rx, Duration::from_secs(2), |b| changed(b).contains(&"docs/plan.md".to_string()));
        assert!(changed(&got).contains(&"docs/plan.md".to_string()), "{got:?}");
    }

    #[test]
    fn watch_dirs_caps_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "a/\nb/\nc/\n").unwrap();
        for d in ["a", "b", "c", "src/ui"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let dirs = watch_dirs(root, root, 2);
        let ignored = ["a", "b", "c"].iter().filter(|d| dirs.contains(&root.join(d))).count();
        assert_eq!(ignored, 2, "{dirs:?}");
        for d in [root.to_path_buf(), root.join("src"), root.join("src").join("ui")] {
            assert!(dirs.contains(&d), "{dirs:?}");
        }
    }

    #[test]
    fn rules_change_sets_flag() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "a/\n").unwrap();
        let (_w, rx) = start(dir.path());
        std::fs::write(dir.path().join(".gitignore"), "a/\nb/\n").unwrap();
        let got = wait_for(&rx, Duration::from_secs(2), |b| b.iter().any(|x| x.rules));
        assert!(got.iter().any(|b| b.rules), "{got:?}");
    }

    #[test]
    fn is_rules_file_names() {
        for p in [".gitignore", "src/.ignore", ".rgignore", ".git/info/exclude"] {
            assert!(is_rules_file(Path::new(p)), "{p}");
        }
        for p in ["a.rs", ".git/HEAD", "gitignore"] {
            assert!(!is_rules_file(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn watch_dirs_enter_tracked_hidden_name() {
        // 管理している vendor/ の中の、外したフォルダも見張る。
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "logs/\n").unwrap();
        std::fs::create_dir_all(root.join("vendor").join("logs")).unwrap();
        let dirs = watch_dirs(root, root, 10);
        assert!(dirs.contains(&root.join("vendor").join("logs")), "{dirs:?}");
    }

    #[test]
    fn rules_change_rebuilds_watch_set() {
        // node_modules を外す行を消したら、node_modules は外していないフォルダになり、見張る。
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        let (_w, rx) = start(root);
        std::fs::write(root.join(".gitignore"), "\n").unwrap();
        let _ = wait_for(&rx, Duration::from_secs(2), |b| b.iter().any(|x| x.rules));
        std::fs::write(root.join("node_modules").join("a.js"), "x").unwrap();
        let got = wait_for(&rx, Duration::from_secs(2), |b| changed(b).contains(&"node_modules/a.js".to_string()));
        assert!(changed(&got).contains(&"node_modules/a.js".to_string()), "{got:?}");
    }

    #[test]
    fn hidden_new_dir_in_ignored_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "docs/\n").unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let (_w, rx) = start(root);
        std::fs::create_dir_all(root.join("docs").join("node_modules")).unwrap();
        std::fs::write(root.join("docs").join("a.md"), "x").unwrap();
        let got = wait_for(&rx, Duration::from_secs(2), |b| changed(b).contains(&"docs/a.md".to_string()));
        assert!(changed(&got).contains(&"docs/a.md".to_string()), "{got:?}");
        assert!(!changed(&got).iter().any(|p| p.contains("node_modules")), "{got:?}");
    }


    #[test]
    fn watch_dirs_prefers_shallow() {
        // 深い生成物より、浅い外したフォルダ（docs/superpowers など）を先に見張りに入れる。
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "a/\nb/\n").unwrap();
        std::fs::create_dir_all(root.join("a").join("x").join("y").join("z")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let dirs = watch_dirs(root, root, 2);
        assert!(dirs.contains(&root.join("a")) && dirs.contains(&root.join("b")), "{dirs:?}");
    }

    #[test]
    fn hidden_component_names() {
        assert!(has_hidden_component(Path::new("target/debug/x")));
        assert!(has_hidden_component(Path::new("web/node_modules/a.js")));
        assert!(!has_hidden_component(Path::new("docs/superpowers/plan.md")));
    }

    #[test]
    fn forgetting_a_dir_frees_its_subtree() {
        let mut set = WatchSet { dirs: HashSet::new(), ignored: HashSet::new() };
        for d in ["/r/a", "/r/a/b", "/r/ab", "/r/c"] {
            set.dirs.insert(PathBuf::from(d));
            set.ignored.insert(PathBuf::from(d));
        }
        set.forget(Path::new("/r/a"));
        let mut left: Vec<_> = set.ignored.iter().cloned().collect();
        left.sort();
        assert_eq!(left, vec![PathBuf::from("/r/ab"), PathBuf::from("/r/c")]);
        assert_eq!(set.dirs.len(), 2);
    }

    /// Linux 以外は、見張りの数を数えないので、外したフォルダの数に関わらず届く（このリポジトリの target-dev など）。
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn ignored_dir_is_reported_beyond_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "big/\ndocs/\n").unwrap();
        for i in 0..IGNORED_DIRS_MAX + 10 {
            std::fs::create_dir_all(root.join("big").join(i.to_string())).unwrap();
        }
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let (_w, rx) = start(root);
        std::fs::create_dir_all(root.join("docs").join("new")).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        std::fs::write(root.join("docs").join("new").join("a.md"), "x").unwrap();
        let got = wait_for(&rx, Duration::from_secs(3), |b| changed(b).contains(&"docs/new/a.md".to_string()));
        assert!(changed(&got).contains(&"docs/new/a.md".to_string()), "{got:?}");
    }

    #[test]
    fn batch_roundtrips_as_json_line() {
        let b = WatchBatch { changed: vec![RelPath::new("a/b c.rs").unwrap()], git: true, rules: true };
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
