//! プロジェクトごとの「生きている」状態。ファイルへのアクセス（ProjectFs）、監視、Git の状態を持つ。
//! 監視の通知を受けて、開いているファイルの読み直し、Git の状態とブランチ名の読み直しを行う。

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use fude_core::workspace::{ProjectId, ProjectRoot};
use fude_fs::git::{GitMark, GitStatus};
use fude_fs::watch::WatchBatch;
use fude_fs::{ProjectFs, RelPath};
use futures::StreamExt;
use gpui_kit::*;

use crate::workspace_view::WorkspaceView;
use crate::wsl_host::{WslHosts, project_fs};

/// Git の状態の読み直しをまとめる時間。
const STATUS_DELAY: Duration = Duration::from_millis(300);

#[derive(Default)]
pub(crate) struct ProjectLive {
    pub status: Option<GitStatus>,
    /// Git の状態を 1 回でも読んだ（status が None でも、リポジトリでないとわかった）。
    pub status_loaded: bool,
    pub fs: Option<Arc<dyn ProjectFs>>,
    /// gitignore を尊重したファイルの一覧（⌘P 用）。パレットを開くたびに裏で読み直す。
    pub files: Option<Arc<Vec<String>>>,
    files_task: Option<Task<()>>,
    /// 監視（LocalWatcher か WslWatcher）。捨てると止まる。
    _watcher: Option<Box<dyn Any>>,
    _events: Option<Task<()>>,
    status_task: Option<Task<()>>,
}

/// 監視を始める。止まった監視（WSL の wsl.exe が終わったなど）は WslWatcher が起動し直す。
fn start_watcher(
    root: &ProjectRoot,
    hosts: &WslHosts,
    heavy: fude_fs::ignored::HeavyDirs,
    on_batch: Box<dyn Fn(WatchBatch) + Send>,
) -> std::io::Result<Box<dyn Any>> {
    match root {
        ProjectRoot::Local(path) => Ok(Box::new(fude_fs::watch::LocalWatcher::start(path.clone(), heavy, on_batch)?)),
        #[cfg(windows)]
        ProjectRoot::Wsl { distro, path } => {
            Ok(Box::new(fude_fs::wsl::WslWatcher::start(distro.clone(), hosts.bin().to_string(), path.clone(), heavy, on_batch)))
        }
        #[cfg(not(windows))]
        ProjectRoot::Wsl { .. } => {
            let _ = hosts;
            Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "WSL は Windows だけ"))
        }
    }
}

impl WorkspaceView {
    /// 開いているプロジェクトに ProjectLive を用意し、閉じたプロジェクトのものを捨てる。描画のたびに呼んでよい。
    pub(crate) fn sync_live(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<(ProjectId, ProjectRoot)> = self.workspace.projects().iter().map(|p| (p.id, p.root.clone())).collect();
        self.live.retain(|id, _| ids.iter().any(|(i, _)| i == id));
        self.searches.retain(|id, _| ids.iter().any(|(i, _)| i == id));
        for (id, root) in ids {
            if self.live.contains_key(&id) {
                continue;
            }
            self.live.insert(id, ProjectLive::default());
            self.start_live(id, root, cx);
        }
    }

    /// ProjectFs を裏で用意し（WSL は配置の確認を待つ）、監視を始め、Git の状態を読む。
    fn start_live(&mut self, project: ProjectId, root: ProjectRoot, cx: &mut Context<Self>) {
        let hosts = self.wsl_hosts.clone();
        let task = cx.background_spawn(async move { project_fs(&root, &hosts).map(|fs| (fs, root, hosts)) });
        cx.spawn(async move |this, cx| {
            let (fs, root, hosts) = match task.await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("プロジェクトのファイルを扱えない: {e}");
                    return;
                }
            };
            let (tx, mut rx) = futures::channel::mpsc::unbounded::<WatchBatch>();
            let watcher = match start_watcher(&root, &hosts, fude_fs::ignored::HeavyDirs::default(), Box::new(move |b| {
                let _ = tx.unbounded_send(b);
            })) {
                Ok(w) => Some(w),
                Err(e) => {
                    eprintln!("監視を始められない: {}: {e}", root.copy_text());
                    None
                }
            };
            this.update(cx, |this, cx| {
                let events = cx.spawn(async move |this, cx| {
                    while let Some(batch) = rx.next().await {
                        if this.update(cx, |this, cx| this.on_watch(project, batch, cx)).is_err() {
                            break;
                        }
                    }
                });
                let Some(live) = this.live.get_mut(&project) else { return };
                live.fs = Some(fs);
                live._watcher = watcher;
                live._events = Some(events);
                this.refresh_status(project, Duration::ZERO, cx);
                this.tree_live_ready(project, cx);
                if this.palette.as_ref().is_some_and(|p| !p.closing) {
                    this.refresh_files(project, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// 監視の通知。開いているファイル（選んでいないタブも含む）が変わっていれば読み直させ、Git の状態を読み直す。
    fn on_watch(&mut self, project: ProjectId, batch: WatchBatch, cx: &mut Context<Self>) {
        let views = self.project_views(project);
        let open: Vec<String> = views.iter().map(|v| v.read(cx).rel.clone()).collect();
        let changed: Vec<&str> = batch.changed.iter().map(|p| p.as_str()).collect();
        let hit = crate::editor_column::watched_views(&changed, &open.iter().map(String::as_str).collect::<Vec<_>>());
        for (view, rel) in views.into_iter().zip(open) {
            if hit.contains(&rel) {
                view.update(cx, |v, cx| v.on_external_change(cx));
            }
        }
        self.reload_pin_if_changed(project, &changed, cx);
        self.on_tree_changed(project, &batch, cx);
        if batch.git {
            self.refresh_branch(project, cx);
        }
        self.refresh_status(project, STATUS_DELAY, cx);
    }

    /// Git の状態を delay のあとに読み直す。続けて呼ばれたら最後の呼び出しから数え直す。
    pub(crate) fn refresh_status(&mut self, project: ProjectId, delay: Duration, cx: &mut Context<Self>) {
        let Some(fs) = self.live.get(&project).and_then(|l| l.fs.clone()) else { return };
        let task = cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let status = cx.background_spawn(async move { fs.git_status() }).await;
            this.update(cx, |this, cx| {
                let status = status.unwrap_or_else(|e| {
                    eprintln!("Git の状態を読めない: {e}");
                    None
                });
                if let Some(live) = this.live.get_mut(&project) {
                    live.status = status;
                    live.status_loaded = true;
                }
                this.apply_marks(project, cx);
                cx.notify();
            })
            .ok();
        });
        if let Some(live) = self.live.get_mut(&project) {
            live.status_task = Some(task);
        }
    }

    /// 開いているファイル（選んでいないタブも含む）に、新しい Git の記号を渡す。
    fn apply_marks(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        for view in self.project_views(project) {
            let rel = view.read(cx).rel.clone();
            let mark = self.git_mark(project, &rel);
            view.update(cx, |v, cx| v.set_mark(mark, cx));
        }
    }

    /// project の Git の記号。
    pub(crate) fn git_mark(&self, project: ProjectId, rel: &str) -> Option<GitMark> {
        let rel = RelPath::new(rel).ok()?;
        self.live.get(&project)?.status.as_ref()?.mark(&rel)
    }

    /// project の Git の状態をまだ読んでいない。
    pub(crate) fn status_pending(&self, project: ProjectId) -> bool {
        !self.live.get(&project).is_some_and(|l| l.status_loaded)
    }

    /// project のファイルの一覧を裏で読み直す。読み終えたら、開いているパレットの一覧を作り直す。
    /// ProjectFs がまだ無ければ、用意できたとき（tree_live_ready と同じ時点）に読む。
    pub(crate) fn refresh_files(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let Some(live) = self.live.get_mut(&project) else { return };
        let Some(fs) = live.fs.clone() else { return };
        let task = cx.background_spawn(async move { fs.walk_files() });
        live.files_task = Some(cx.spawn(async move |this, cx| {
            let files = match task.await {
                Ok(files) => palette_paths(files),
                Err(e) => {
                    eprintln!("ファイルの一覧を読めない: {e}");
                    return;
                }
            };
            this.update(cx, |this, cx| {
                let Some(live) = this.live.get_mut(&project) else { return };
                live.files = Some(Arc::new(files));
                live.files_task = None;
                if this.palette.as_ref().is_some_and(|p| !p.closing) {
                    this.refresh_palette(cx);
                }
            })
            .ok();
        }));
    }

    /// project の ProjectFs（用意できていれば）。
    pub(crate) fn live_fs(&self, project: ProjectId) -> Option<Arc<dyn ProjectFs>> {
        self.live.get(&project)?.fs.clone()
    }
}

/// ⌘P の一覧。外していないものの後ろに外したものを続ける（あいまい検索は同じ点数なら並びの順なので、
/// 外したものが後ろに来る）。
pub(crate) fn palette_paths(files: fude_fs::WalkedFiles) -> Vec<String> {
    files.tracked.into_iter().chain(files.ignored).map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::palette_paths;
    use fude_fs::{RelPath, WalkedFiles};

    #[test]
    fn palette_paths_put_ignored_last() {
        let files = WalkedFiles { tracked: vec![RelPath::new("b").unwrap()], ignored: vec![RelPath::new("a").unwrap()] };
        assert_eq!(palette_paths(files), vec!["b".to_string(), "a".to_string()]);
    }
}
