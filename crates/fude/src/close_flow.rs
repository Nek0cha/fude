//! 閉じる操作（設計書 §5.12）。閉じるキーの行き先を決め、動いているターミナルがあれば確認してから閉じる。

use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fude_core::close::{CloseTarget, FocusArea, close_target};
use fude_core::workspace::{PaneId, ProjectId, ShellKind};
use gpui_kit::*;

use crate::dialog::{ButtonRole, Dialog, DialogButton, error_text};
use crate::file_view::FileView;
use crate::workspace_view::{WorkspaceView, terminal_id};

/// WSL のターミナルの稼働の確認を待つ時間。過ぎたら動いているとみなす（設計書 §5.12）。
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_millis(300);

/// 1 つのペインの稼働の調べ方。
pub(crate) enum BusyCheck {
    Known(bool),
    /// 裏で呼ぶ（WSL の fude serve への問い合わせ）。
    Remote(Box<dyn FnOnce() -> io::Result<bool> + Send>),
}

/// 動いているペイン（並びは入力のまま）。Remote は並べて呼び、BUSY_TIMEOUT までに答えが無いか Err なら動いているとみなす。
/// 裏のスレッドから呼ぶ（最長 BUSY_TIMEOUT 待つ）。
pub(crate) fn busy_panes(checks: Vec<(PaneId, BusyCheck)>) -> Vec<PaneId> {
    let (tx, rx) = mpsc::channel::<(usize, bool)>();
    let mut answers: Vec<Option<bool>> = Vec::with_capacity(checks.len());
    let mut pending = 0;
    let ids: Vec<PaneId> = checks.iter().map(|(id, _)| *id).collect();
    for (i, (_, check)) in checks.into_iter().enumerate() {
        match check {
            BusyCheck::Known(b) => answers.push(Some(b)),
            BusyCheck::Remote(f) => {
                answers.push(None);
                pending += 1;
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send((i, f().unwrap_or(true)));
                });
            }
        }
    }
    let deadline = Instant::now() + BUSY_TIMEOUT;
    while pending > 0 {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
        match rx.recv_timeout(left) {
            Ok((i, b)) => {
                answers[i] = Some(b);
                pending -= 1;
            }
            Err(_) => break,
        }
    }
    ids.into_iter().zip(answers).filter(|(_, a)| a.unwrap_or(true)).map(|(id, _)| id).collect()
}

/// 確認ダイアログが出ている間は、閉じるキーを受け付けない（連打で意図せず閉じないため）。
pub(crate) fn should_handle_close(dialog_open: bool) -> bool {
    !dialog_open
}

impl WorkspaceView {
    /// 今フォーカスのある場所。
    pub(crate) fn focus_area(&self, window: &Window, cx: &App) -> FocusArea {
        if self.workspace.projects().is_empty() {
            return FocusArea::Home;
        }
        if let Some(view) = self.active_panel_view()
            && view.read(cx).focus_handle().contains_focused(window, cx)
        {
            return FocusArea::Editor;
        }
        if self.tree.open && self.tree.focus.contains_focused(window, cx) {
            return FocusArea::Tree;
        }
        for (id, view) in self.terminal_views() {
            if view.read(cx).focus_handle().contains_focused(window, cx) {
                return FocusArea::Terminal(id);
            }
        }
        FocusArea::Other
    }

    /// 閉じるキー。
    pub(crate) fn close_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !should_handle_close(self.dialog_open()) {
            return;
        }
        let focus = self.focus_area(window, cx);
        let target = close_target(&self.workspace, focus, self.tree.open);
        self.request_close(target, window, cx);
    }

    /// target を閉じる。ターミナルなら動いているかを確かめ、プロジェクトなら未保存の変更も確かめてから閉じる。
    pub(crate) fn request_close(&mut self, target: CloseTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            CloseTarget::Nothing => {}
            // 計画5の 1 ファイルの間は、タブを閉じる＝右パネルを閉じる（Task 7 でタブごとに閉じる）。
            CloseTarget::EditorTab(_) => self.close_panel(window, cx),
            CloseTarget::Tree => self.close_tree(window, cx),
            CloseTarget::Pane(pane) => self.confirm_busy(vec![pane], window, cx, move |this, _, cx| {
                this.workspace.close_pane(pane);
                this.refocus_pane();
                cx.notify();
            }),
            CloseTarget::Tab(tab) => {
                let panes = self
                    .workspace
                    .projects()
                    .iter()
                    .flat_map(|p| &p.tabs)
                    .find(|t| t.id == tab)
                    .map(|t| t.panes.iter().map(|p| p.id).collect())
                    .unwrap_or_default();
                self.confirm_busy(panes, window, cx, move |this, _, cx| {
                    this.workspace.close_tab(tab);
                    this.refocus_pane();
                    cx.notify();
                });
            }
            CloseTarget::Project(project) => self.request_close_project(project, window, cx),
        }
    }

    pub(crate) fn request_close_project(&mut self, project: ProjectId, window: &mut Window, cx: &mut Context<Self>) {
        let panes: Vec<PaneId> = self
            .workspace
            .projects()
            .iter()
            .filter(|p| p.id == project)
            .flat_map(|p| &p.tabs)
            .flat_map(|t| t.panes.iter().map(|p| p.id))
            .collect();
        self.confirm_unsaved(Some(project), window, cx, move |this, window, cx| {
            this.confirm_busy(panes, window, cx, move |this, window, cx| {
                this.workspace.close_project(project);
                this.branches.remove(&project);
                this.panels.remove(&project);
                this.refresh_active_branch(cx);
                this.restore_focus(window, cx);
                this.after_project_closed(cx);
                cx.notify();
            });
        });
    }

    /// アプリを終える（ウィンドウの ×）。未保存の変更と、動いているターミナルを確かめてから閉じる。
    pub(crate) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !should_handle_close(self.dialog_open()) {
            return;
        }
        let panes: Vec<PaneId> =
            self.workspace.projects().iter().flat_map(|p| &p.tabs).flat_map(|t| t.panes.iter().map(|p| p.id)).collect();
        self.confirm_unsaved(None, window, cx, move |this, window, cx| {
            this.confirm_busy(panes, window, cx, |this, window, _| {
                this.allow_close = true;
                this.save_now();
                window.remove_window();
            });
        });
    }

    /// project（None は全プロジェクト）の右パネルに未保存の変更があれば、保存・保存しない・キャンセルを聞く。
    fn confirm_unsaved(
        &mut self,
        project: Option<ProjectId>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>) + 'static,
    ) {
        let dirty: Vec<_> = self
            .panels
            .iter()
            .filter(|(id, _)| project.is_none_or(|p| p == **id))
            .filter_map(|(_, s)| s.view.clone())
            .filter(|v| v.read(cx).is_dirty())
            .collect();
        if dirty.is_empty() {
            then(self, window, cx);
            return;
        }
        let message = match dirty.as_slice() {
            [one] => format!("{} の変更を保存しますか？", one.read(cx).rel.rsplit('/').next().unwrap_or_default()),
            many => format!("保存していない変更があります（{} 個）。保存しますか？", many.len()),
        };
        let dialog = Dialog {
            message,
            buttons: vec![
                DialogButton { label: "保存", role: ButtonRole::Primary },
                DialogButton { label: "保存しない", role: ButtonRole::Danger },
                DialogButton { label: "キャンセル", role: ButtonRole::Cancel },
            ],
            on_choose: Box::new(move |i, this, window, cx| match i {
                0 => this.save_then(dirty, window, cx, then),
                1 => then(this, window, cx),
                _ => {}
            }),
        };
        self.show_dialog(dialog, window, cx);
    }

    /// views を保存し、すべて書き終えてから then を呼ぶ。1 つでも失敗したら then は呼ばず、失敗を知らせる
    /// （閉じる・終えるを取りやめ、未保存の中身を残す）。
    pub(crate) fn save_then(
        &mut self,
        views: Vec<Entity<FileView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>) + 'static,
    ) {
        let tasks: Vec<(String, Task<io::Result<()>>)> =
            views.iter().map(|v| (v.read(cx).rel.clone(), v.update(cx, |v, cx| v.save(cx)))).collect();
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let mut failures = Vec::new();
            for (rel, task) in tasks {
                if let Err(e) = task.await {
                    failures.push(error_text("保存できません", &rel, &e));
                }
            }
            cx.update_window(window_handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    if failures.is_empty() {
                        then(this, window, cx);
                    } else {
                        this.show_error(failures.join("\n\n"), window, cx);
                    }
                })
                .ok();
            })
            .ok();
        })
        .detach();
    }

    /// panes のどれかで何かが動いていれば、確かめてから then を呼ぶ。設定で無効なら確かめない。
    fn confirm_busy(
        &mut self,
        panes: Vec<PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>) + 'static,
    ) {
        if !self.settings.confirm_close_running || panes.is_empty() {
            then(self, window, cx);
            return;
        }
        let checks: Vec<(PaneId, BusyCheck)> = panes.iter().map(|&p| (p, self.busy_check(p, cx))).collect();
        let task = cx.background_spawn(async move { busy_panes(checks) });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let busy = task.await;
            cx.update_window(window_handle, |_, window, cx| {
                this.update(cx, |this, cx| {
                    if busy.is_empty() {
                        then(this, window, cx);
                        return;
                    }
                    let dialog = Dialog {
                        message: format!("動いているターミナルがあります（{} 個）。閉じますか？", busy.len()),
                        buttons: vec![
                            DialogButton { label: "キャンセル", role: ButtonRole::Cancel },
                            DialogButton { label: "閉じる", role: ButtonRole::Danger },
                        ],
                        on_choose: Box::new(move |i, this, window, cx| {
                            if i == 1 {
                                then(this, window, cx);
                            }
                        }),
                    };
                    this.show_dialog(dialog, window, cx);
                })
                .ok();
            })
            .ok();
        })
        .detach();
    }

    /// pane の稼働の調べ方。ネイティブは PTY から、WSL は fude serve に FUDE_TERMINAL_ID で聞く。
    fn busy_check(&self, pane: PaneId, cx: &App) -> BusyCheck {
        let Some(view) = self.terminal_view(pane) else { return BusyCheck::Known(false) };
        let Some(session) = view.read(cx).session.clone() else { return BusyCheck::Known(false) };
        if let Some(b) = session.has_foreground_job() {
            return BusyCheck::Known(b);
        }
        let kind = self
            .workspace
            .projects()
            .iter()
            .flat_map(|p| &p.tabs)
            .flat_map(|t| &t.panes)
            .find(|p| p.id == pane)
            .map(|p| p.kind.clone());
        match kind {
            #[cfg(windows)]
            Some(ShellKind::Wsl { distro }) => {
                let hosts = self.wsl_hosts.clone();
                let id = terminal_id(pane);
                BusyCheck::Remote(Box::new(move || fude_fs::busy(&*hosts.client(&distro)?, &id)))
            }
            _ => {
                let _ = terminal_id;
                BusyCheck::Known(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BusyCheck, busy_panes, should_handle_close};
    use fude_core::workspace::{PaneId, Workspace};
    use std::io;
    use std::time::{Duration, Instant};

    /// テスト用に、ペインの id を 3 つ作る。
    fn panes() -> Vec<PaneId> {
        let mut ws = Workspace::default();
        ws.open_project(fude_core::workspace::ProjectRoot::Local("/w/a".into()));
        ws.split_active_tab();
        ws.new_tab(fude_core::workspace::ShellKind::Native);
        ws.active_project().unwrap().tabs.iter().flat_map(|t| t.panes.iter().map(|p| p.id)).collect()
    }

    #[test]
    fn busy_timeout_counts_as_busy() {
        let p = panes();
        let slow = BusyCheck::Remote(Box::new(|| {
            std::thread::sleep(Duration::from_secs(2));
            Ok(false)
        }));
        let err = BusyCheck::Remote(Box::new(|| Err(io::Error::other("x"))));
        let idle = BusyCheck::Known(false);
        let t = Instant::now();
        assert_eq!(busy_panes(vec![(p[0], slow), (p[1], err), (p[2], idle)]), vec![p[0], p[1]]);
        assert!(t.elapsed() < Duration::from_millis(900), "{:?}", t.elapsed());
    }

    #[test]
    fn remote_answers_are_used() {
        let p = panes();
        let yes = BusyCheck::Remote(Box::new(|| Ok(true)));
        let no = BusyCheck::Remote(Box::new(|| Ok(false)));
        assert_eq!(busy_panes(vec![(p[0], yes), (p[1], no), (p[2], BusyCheck::Known(true))]), vec![p[0], p[2]]);
    }

    #[test]
    fn close_key_ignored_while_dialog_open() {
        assert!(!should_handle_close(true));
        assert!(should_handle_close(false));
    }
}
