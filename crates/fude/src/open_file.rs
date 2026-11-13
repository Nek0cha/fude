//! ファイルを開くときに、どのプロジェクトで開くかを決める（設計書 §5.1 の A 案）。
//! 開いているプロジェクトの中ならそこで、そうでなければ Git のルート（無ければファイルのあるフォルダ）を
//! プロジェクトにして、ターミナルを起動せずにエディタだけで開く。

use std::path::{Path, PathBuf};

use fude_core::location::{FileLocation, find_project_root};
use fude_core::workspace::{OpenMode, ProjectId, ProjectRoot, Workspace};
use gpui_kit::*;

use crate::workspace_view::WorkspaceView;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OpenPlan {
    /// 開いているプロジェクトで開く。
    Existing { project: ProjectId, rel: String },
    /// root を新しいプロジェクトとして（ターミナルなしで）開く。
    New { root: ProjectRoot, rel: String },
    /// WSL のファイル。ルートを fude serve に探させてから New にする。
    FindRemoteRoot { distro: String, file: String },
}

/// local_root はローカルのファイルのプロジェクトのルート（`.git` を持つ最初の祖先、無ければ親）を返す関数。
pub(crate) fn plan_open(
    ws: &Workspace,
    file: &FileLocation,
    root_hint: Option<&ProjectRoot>,
    local_root: &dyn Fn(&Path) -> PathBuf,
) -> OpenPlan {
    if let Some((project, rel)) = ws.project_for_file(file) {
        return OpenPlan::Existing { project, rel };
    }
    if let Some(hint) = root_hint
        && let Some(rel) = file.rel_within(hint)
    {
        return OpenPlan::New { root: hint.clone(), rel };
    }
    match file {
        FileLocation::Local(path) => {
            let root = ProjectRoot::from_host_path(local_root(path));
            let rel = file.rel_within(&root).unwrap_or_else(|| {
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            });
            OpenPlan::New { root, rel }
        }
        FileLocation::Wsl { distro, path } => OpenPlan::FindRemoteRoot { distro: distro.clone(), file: path.clone() },
    }
}

impl WorkspaceView {
    /// ファイルを開く（引数、2 つ目の起動、ドロップ、ターミナルのリンクから）。line は 1 始まり。
    pub(crate) fn open_file(
        &mut self,
        file: FileLocation,
        root_hint: Option<ProjectRoot>,
        at: Option<(u32, Option<u32>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plan = plan_open(&self.workspace, &file, root_hint.as_ref(), &|p| {
            find_project_root(p, &|d| d.join(".git").exists())
        });
        match plan {
            OpenPlan::Existing { project, rel } => {
                self.workspace.activate_project(project);
                self.refresh_active_branch(cx);
                self.open_in_panel_at(project, rel, at, window, cx);
            }
            OpenPlan::New { root, rel } => self.open_file_in_new_project(root, rel, at, window, cx),
            OpenPlan::FindRemoteRoot { distro, file: path } => {
                let task = self.find_remote_root(distro.clone(), path.clone(), cx);
                let window_handle = window.window_handle();
                cx.spawn(async move |this, cx| {
                    let root = match task.await {
                        Ok(root) => root,
                        Err(e) => {
                            eprintln!("WSL のファイルのプロジェクトを決められない: {path}: {e}");
                            return;
                        }
                    };
                    let root = ProjectRoot::from_wsl_path(&distro, &root);
                    let file = FileLocation::Wsl { distro, path };
                    cx.update_window(window_handle, |_, window, cx| {
                        this.update(cx, |this, cx| this.open_file(file, Some(root), at, window, cx)).ok();
                    })
                    .ok();
                })
                .detach();
            }
        }
        cx.notify();
    }

    fn open_file_in_new_project(
        &mut self,
        root: ProjectRoot,
        rel: String,
        at: Option<(u32, Option<u32>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.touch_recent(&root, cx);
        let project = self.workspace.open_project_with(root, None, OpenMode::EditorOnly);
        self.refresh_active_branch(cx);
        self.open_in_panel_at(project, rel, at, window, cx);
    }

    fn open_in_panel_at(
        &mut self,
        project: ProjectId,
        rel: String,
        at: Option<(u32, Option<u32>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = at.map(|_| fude_core::panel::FileMode::Full);
        // 開き直し（同じファイル）は open_in_panel の中ですぐ行を移すため、先に入れておく。
        if let Some((line, col)) = at {
            self.pending_goto = Some((project, line, col));
        }
        self.open_in_panel(project, rel, mode, window, cx);
    }

    #[cfg(windows)]
    fn find_remote_root(&self, distro: String, file: String, cx: &mut Context<Self>) -> Task<std::io::Result<String>> {
        let hosts = self.wsl_hosts.clone();
        cx.background_spawn(async move { fude_fs::find_root(&*hosts.client(&distro)?, &file) })
    }

    #[cfg(not(windows))]
    fn find_remote_root(&self, _: String, _: String, _: &mut Context<Self>) -> Task<std::io::Result<String>> {
        Task::ready(Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "WSL のファイルは Windows だけで開ける")))
    }

    /// ウィンドウに落とされたもの。フォルダはプロジェクトとして、ファイルはエディタで開く。
    pub(crate) fn open_dropped(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        for path in paths.paths() {
            let Ok(canonical) = std::fs::canonicalize(path) else { continue };
            let canonical = fude_core::paths::simplify_verbatim(canonical);
            if canonical.is_dir() {
                self.open_project(ProjectRoot::from_host_path(canonical), None, cx);
            } else {
                self.open_file(FileLocation::from_host_path(canonical), None, None, window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenPlan, plan_open};
    use fude_core::location::FileLocation;
    use fude_core::workspace::{ProjectRoot, Workspace};
    use std::path::PathBuf;

    fn local(p: &str) -> ProjectRoot {
        ProjectRoot::Local(PathBuf::from(p))
    }

    fn wsl(d: &str, p: &str) -> ProjectRoot {
        ProjectRoot::Wsl { distro: d.into(), path: p.into() }
    }

    #[test]
    fn plan_open_prefers_open_project_then_hint_then_git_root() {
        let mut ws = Workspace::default();
        ws.open_project(local("/w/app"));
        let f = FileLocation::Local(PathBuf::from("/w/app/src/a.rs"));
        assert!(matches!(plan_open(&ws, &f, None, &|_| unreachable!()), OpenPlan::Existing { ref rel, .. } if rel == "src/a.rs"));

        let g = FileLocation::Local(PathBuf::from("/x/repo/b.rs"));
        assert_eq!(
            plan_open(&ws, &g, None, &|_| PathBuf::from("/x/repo")),
            OpenPlan::New { root: local("/x/repo"), rel: "b.rs".into() }
        );

        let w = FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/x/p/c.rs".into() };
        assert_eq!(
            plan_open(&ws, &w, None, &|_| unreachable!()),
            OpenPlan::FindRemoteRoot { distro: "Ubuntu".into(), file: "/home/x/p/c.rs".into() }
        );
        let hint = wsl("Ubuntu", "/home/x/p");
        assert_eq!(plan_open(&ws, &w, Some(&hint), &|_| unreachable!()), OpenPlan::New { root: hint.clone(), rel: "c.rs".into() });
    }

    #[test]
    fn hint_outside_file_falls_back() {
        let ws = Workspace::default();
        let f = FileLocation::Local(PathBuf::from("/x/repo/b.rs"));
        // ヒントがファイルを含まないなら、ヒントは使わない。
        assert_eq!(
            plan_open(&ws, &f, Some(&local("/other")), &|_| PathBuf::from("/x/repo")),
            OpenPlan::New { root: local("/x/repo"), rel: "b.rs".into() }
        );
    }
}
