//! 右パネル（設計書 §4.1）。プロジェクトごとに、ファイルの表示と履歴を持つ。
//! タブがあるときは「ターミナル｜右パネル」に並べ、タブが 0 枚ならメイン領域の全体を使う（§4.1.1）。

use std::sync::Arc;

use fude_core::panel::{FileMode, PanelHistory, PanelItem};
use fude_core::workspace::ProjectId;
use fude_fs::ProjectFs;
use fude_fs::git::GitMark;
use gpui_kit::*;

use crate::dialog::{ButtonRole, Dialog, DialogButton};
use crate::file_view::{FileView, FileViewEvent, initial_mode};
use crate::workspace_view::WorkspaceView;
use crate::wsl_host::project_fs;

/// 右パネルの幅。メイン領域の 45%（最小 420px、最大 900px）。メイン領域が狭い（620px 未満）なら全体。
pub(crate) fn panel_width(main_w: Pixels) -> Pixels {
    if main_w < px(620.) {
        return main_w;
    }
    (main_w * 0.45).clamp(px(420.), px(900.))
}

/// プロジェクトごとの右パネルの状態。
#[derive(Default)]
pub(crate) struct PanelSlot {
    pub history: PanelHistory,
    pub view: Option<Entity<FileView>>,
    /// 開くのを待っているファイル（ProjectFs を裏で用意している間）。
    pub loading: Option<String>,
    _sub: Option<Subscription>,
}

impl PanelSlot {
    pub fn is_open(&self) -> bool {
        self.view.is_some()
    }
}

/// 置き換える前に、未保存の変更をどうするか。
enum Replace {
    Save,
    Discard,
}

impl WorkspaceView {
    /// 選択中のプロジェクトの右パネルが開いているか。
    pub(crate) fn panel_open(&self) -> bool {
        self.workspace.active_project().and_then(|p| self.panels.get(&p.id)).is_some_and(PanelSlot::is_open)
    }

    pub(crate) fn active_panel_view(&self) -> Option<Entity<FileView>> {
        let p = self.workspace.active_project()?;
        self.panels.get(&p.id)?.view.clone()
    }

    /// project の右パネルに rel を開く。今の表示に未保存の変更があれば、保存・保存しない・キャンセルを聞く。
    /// mode が None なら Git の状態から決める（Task 16 で状態が入るまでは全体）。
    pub(crate) fn open_in_panel(
        &mut self,
        project: ProjectId,
        rel: String,
        mode: Option<FileMode>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mark = self.git_mark(project, &rel);
        let item = PanelItem { rel, mode: mode.unwrap_or_else(|| initial_mode(mark)) };
        let next = item.rel.clone();
        self.confirm_leave(project, Some(&next), window, cx, move |this, window, cx| {
            this.panels.entry(project).or_default().history.open(item.clone());
            this.show_item(project, item, mark, window, cx);
        });
    }

    /// 戻る（delta = -1）／進む（+1）。
    pub(crate) fn panel_navigate(&mut self, project: ProjectId, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slot) = self.panels.get(&project) else { return };
        let target = {
            let mut h = slot.history.clone();
            let item = if delta < 0 { h.back().cloned() } else { h.forward().cloned() };
            item
        };
        let Some(item) = target else { return };
        let next = item.rel.clone();
        self.confirm_leave(project, Some(&next), window, cx, move |this, window, cx| {
            let slot = this.panels.entry(project).or_default();
            if delta < 0 {
                slot.history.back();
            } else {
                slot.history.forward();
            }
            let mark = this.git_mark(project, &item.rel);
            this.show_item(project, item, mark, window, cx);
        });
    }

    /// 選択中のプロジェクトの右パネルを閉じる（未保存なら確認する）。
    pub(crate) fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        self.confirm_leave(project, None, window, cx, move |this, window, cx| {
            if let Some(slot) = this.panels.get_mut(&project) {
                slot.view = None;
                slot.loading = None;
                slot._sub = None;
            }
            this.workspace.set_open_file(project, None);
            this.restore_focus(window, cx);
            cx.notify();
        });
    }

    /// project の右パネルの今の表示に未保存の変更があり、next（None は閉じる）と違うファイルなら、聞いてから then を呼ぶ。
    fn confirm_leave(
        &mut self,
        project: ProjectId,
        next: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>) + 'static,
    ) {
        let view = self.panels.get(&project).and_then(|s| s.view.clone());
        let Some(view) = view.filter(|v| v.read(cx).is_dirty() && next != Some(v.read(cx).rel.as_str())) else {
            then(self, window, cx);
            return;
        };
        let name = view.read(cx).rel.rsplit('/').next().unwrap_or_default().to_string();
        let dialog = Dialog {
            message: format!("{name} の変更を保存しますか？"),
            buttons: vec![
                DialogButton { label: "保存", role: ButtonRole::Primary },
                DialogButton { label: "保存しない", role: ButtonRole::Danger },
                DialogButton { label: "キャンセル", role: ButtonRole::Cancel },
            ],
            on_choose: Box::new(move |i, this, window, cx| {
                let choice = match i {
                    0 => Replace::Save,
                    1 => Replace::Discard,
                    _ => return,
                };
                if let Replace::Save = choice {
                    view.update(cx, |v, cx| v.save(cx));
                }
                then(this, window, cx);
            }),
        };
        self.show_dialog(dialog, window, cx);
    }

    /// item を表示する。ProjectFs を裏で用意してから FileView を作る（WSL は fude serve の起動を待つことがある）。
    fn show_item(&mut self, project: ProjectId, item: PanelItem, mark: Option<GitMark>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.projects().iter().find(|p| p.id == project).map(|p| p.root.clone()) else {
            return;
        };
        self.workspace.set_open_file(project, Some(item.rel.clone()));
        let slot = self.panels.entry(project).or_default();
        slot.loading = Some(item.rel.clone());
        let hosts = self.wsl_hosts.clone();
        let task = cx.background_spawn(async move { project_fs(&root, &hosts) });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let fs: Arc<dyn ProjectFs> = match task.await {
                Ok(fs) => fs,
                Err(e) => {
                    eprintln!("ファイルを開けない: {}: {e}", item.rel);
                    return;
                }
            };
            cx.update_window(window_handle, |_, window, cx| {
                this.update(cx, |this, cx| this.install_view(project, item, mark, fs, window, cx)).ok();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn install_view(
        &mut self,
        project: ProjectId,
        item: PanelItem,
        mark: Option<GitMark>,
        fs: Arc<dyn ProjectFs>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slot = self.panels.entry(project).or_default();
        // 待っている間に、別のファイルを開いたか閉じた。
        if slot.loading.as_deref() != Some(item.rel.as_str()) {
            return;
        }
        slot.loading = None;
        let (can_back, can_forward) = (slot.history.can_back(), slot.history.can_forward());
        let view = cx.new(|cx| {
            let mut v = FileView::new(fs, item.rel.clone(), item.mode, mark, cx);
            v.set_nav(can_back, can_forward);
            v
        });
        let sub = cx.subscribe_in(&view, window, move |this, _, ev: &FileViewEvent, window, cx| match ev {
            FileViewEvent::Close => this.close_panel(window, cx),
            FileViewEvent::Back => this.panel_navigate(project, -1, window, cx),
            FileViewEvent::Forward => this.panel_navigate(project, 1, window, cx),
            FileViewEvent::Dirty(_) | FileViewEvent::Saved => cx.notify(),
        });
        let focus = view.read(cx).focus_handle();
        let slot = self.panels.entry(project).or_default();
        slot.view = Some(view);
        slot._sub = Some(sub);
        if self.workspace.active_project().map(|p| p.id) == Some(project) {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// Git の記号（Task 16 で状態を持つまでは None）。
    pub(crate) fn git_mark(&self, project: ProjectId, rel: &str) -> Option<GitMark> {
        let _ = (project, rel);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::panel_width;
    use gpui_kit::px;

    #[test]
    fn panel_width_rules() {
        assert_eq!(panel_width(px(2000.)), px(900.));
        assert_eq!(panel_width(px(1200.)), px(540.));
        assert_eq!(panel_width(px(800.)), px(420.));
        assert_eq!(panel_width(px(500.)), px(500.));
    }
}
