//! 右パネル（設計書 §4.1）。プロジェクトごとに、ファイルの表示と履歴を持つ。
//! タブがあるときは「ターミナル｜右パネル」に並べ、タブが 0 枚ならメイン領域の全体を使う（§4.1.1）。

use std::sync::Arc;

use fude_core::panel::{FileMode, PanelHistory, PanelItem, renamed_rel};
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

/// 右パネルの幅。share は 1.0 でメイン領域の全体（ターミナルが 0 枚）、0.0 で panel_width（並べた形）。
/// その間は線形に補う（エディタの移動の動き。設計書 §6.3）。
pub(crate) fn panel_target_width(share: f32, main_w: Pixels) -> Pixels {
    let narrow = panel_width(main_w);
    narrow + (main_w - narrow) * share.clamp(0.0, 1.0)
}

/// 右パネルの出入りと、エディタの移動の動き（選択中のプロジェクトのもの）。
pub(crate) struct PanelAnim {
    pub presence: crate::anim::Presence,
    /// 1.0 = メイン領域の全体、0.0 = 並べた幅。
    pub share: fude_core::motion::Fade,
    pub project: Option<ProjectId>,
    /// 閉じて、消える動きの間だけ描き続けるビュー。
    pub closing: Option<Entity<FileView>>,
}

impl Default for PanelAnim {
    fn default() -> Self {
        PanelAnim {
            presence: crate::anim::Presence::hidden(),
            share: fude_core::motion::Fade::settled(0.0),
            project: None,
            closing: None,
        }
    }
}

/// プロジェクトごとの右パネルの状態。
#[derive(Default)]
pub(crate) struct PanelSlot {
    pub history: PanelHistory,
    pub view: Option<Entity<FileView>>,
    /// 開くのを待っているファイル（ProjectFs を裏で用意している間）。
    pub loading: Option<String>,
    /// 次に作る FileView は、Git の状態が届いたら表示の仕方を決め直す。
    pub auto_mode: bool,
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
    /// 描画のたびに呼ぶ。右パネルの出入りと、ターミナルの有無によるエディタの移動の動きを始める。
    pub(crate) fn observe_panel(&mut self, now: std::time::Instant) {
        use crate::theme::motion::{EDITOR_MOVE, PANEL_IN, PANEL_OUT, ease_out_quint};
        let project = self.workspace.active_project().map(|p| p.id);
        let has_tabs = self.workspace.active_project().is_some_and(|p| !p.tabs.is_empty());
        let open = self.panel_open();
        let reduced = self.reduced_motion;
        let target = if has_tabs { 0.0 } else { 1.0 };
        let a = &mut self.panel_anim;
        if a.project != project {
            // プロジェクトを切り替えたときは動かさない。
            a.project = project;
            a.presence = if open { crate::anim::Presence::shown() } else { crate::anim::Presence::hidden() };
            a.share = fude_core::motion::Fade::settled(target);
            a.closing = None;
            return;
        }
        if open && !a.presence.visible() {
            a.closing = None;
            a.presence.show(PANEL_IN, reduced, now);
        } else if !open && a.presence.visible() {
            a.presence.hide(PANEL_OUT, reduced, now);
        }
        if a.presence.is_gone(now) {
            a.closing = None;
        }
        let duration = if reduced || !open { std::time::Duration::ZERO } else { EDITOR_MOVE };
        a.share.animate_to(target, duration, ease_out_quint, now);
    }

    /// 右パネルとエディタの移動が動いている（ターミナルの大きさが変わる）。
    pub(crate) fn panel_animating(&self, now: std::time::Instant) -> bool {
        self.panel_anim.presence.is_animating(now) || self.panel_anim.share.is_animating(now)
    }

    /// メイン領域に描く右パネルのビュー、幅、不透明度。
    pub(crate) fn panel_for_render(&self, main_w: Pixels, now: std::time::Instant) -> Option<(AnyView, Pixels, f32)> {
        let view = self.active_panel_view().or_else(|| self.panel_anim.closing.clone())?;
        let p = &self.panel_anim.presence;
        let w = panel_target_width(self.panel_anim.share.value(now), main_w) * p.travel(now);
        Some((AnyView::from(view), w, p.value(now)))
    }

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
        let auto = mode.is_none() && self.status_pending(project);
        let item = PanelItem { rel, mode: mode.unwrap_or_else(|| initial_mode(mark)) };
        let next = item.rel.clone();
        self.confirm_leave(project, Some(&next), window, cx, move |this, window, cx| {
            let slot = this.panels.entry(project).or_default();
            let new = slot.history.open(item.clone());
            // 今表示しているファイルを開き直したなら、作り直さない（未保存の変更を残す）。
            let same = slot.view.clone().filter(|v| !new && slot.loading.is_none() && v.read(cx).rel == item.rel);
            if let Some(view) = same {
                if let Some(mode) = mode {
                    view.update(cx, |v, cx| v.set_mode(mode, cx));
                }
                if let Some((p, line, col)) = this.pending_goto
                    && p == project
                {
                    this.pending_goto = None;
                    view.update(cx, |v, cx| v.goto(line, col, window, cx));
                }
                window.focus(&view.read(cx).focus_handle(), cx);
                cx.notify();
                return;
            }
            slot.auto_mode = auto;
            this.show_item(project, item, mark, window, cx);
        });
    }

    /// ツリーで from の名前が to に変わった。開いているファイルと履歴のパスを合わせる。
    pub(crate) fn panel_renamed(&mut self, project: ProjectId, from: &str, to: &str, cx: &mut Context<Self>) {
        let Some(slot) = self.panels.get_mut(&project) else { return };
        slot.history.rename(from, to);
        let Some(view) = slot.view.clone() else { return };
        let Some(rel) = renamed_rel(&view.read(cx).rel, from, to) else { return };
        view.update(cx, |v, cx| v.set_rel(rel.clone(), cx));
        set_single_file(&mut self.workspace, project, Some(rel));
        cx.notify();
    }

    /// 戻る（delta = -1）／進む（+1）。
    pub(crate) fn panel_navigate(&mut self, project: ProjectId, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slot) = self.panels.get(&project) else { return };
        let target = {
            let mut h = slot.history.clone();
            if delta < 0 { h.back().cloned() } else { h.forward().cloned() }
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
                // 消える動きの間だけ描き続ける。
                this.panel_anim.closing = slot.view.take();
                slot.loading = None;
                slot._sub = None;
            }
            set_single_file(&mut this.workspace, project, None);
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
                    this.save_then(vec![view], window, cx, then);
                } else {
                    then(this, window, cx);
                }
            }),
        };
        self.show_dialog(dialog, window, cx);
    }

    /// item を表示する。ProjectFs を裏で用意してから FileView を作る（WSL は fude serve の起動を待つことがある）。
    fn show_item(&mut self, project: ProjectId, item: PanelItem, mark: Option<GitMark>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.projects().iter().find(|p| p.id == project).map(|p| p.root.clone()) else {
            return;
        };
        set_single_file(&mut self.workspace, project, Some(item.rel.clone()));
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
        let auto = std::mem::take(&mut slot.auto_mode);
        let view = cx.new(|cx| {
            let mut v = FileView::new(fs, item.rel.clone(), item.mode, mark, cx);
            v.set_nav(can_back, can_forward);
            v.set_auto_mode(auto);
            v
        });
        // 状態がすでに届いていれば、すぐに決め直す。
        if auto && !self.status_pending(project) {
            let mark = self.git_mark(project, &item.rel);
            view.update(cx, |v, cx| v.set_mark(mark, cx));
        }
        let sub = cx.subscribe_in(&view, window, move |this, _, ev: &FileViewEvent, window, cx| match ev {
            FileViewEvent::Close => this.close_panel(window, cx),
            FileViewEvent::Back => this.panel_navigate(project, -1, window, cx),
            FileViewEvent::Forward => this.panel_navigate(project, 1, window, cx),
            FileViewEvent::Dirty | FileViewEvent::Saved => cx.notify(),
            FileViewEvent::SaveFailed(message) => this.show_error(message.clone(), window, cx),
        });
        if let Some((p, line, col)) = self.pending_goto
            && p == project
        {
            self.pending_goto = None;
            view.update(cx, |v, cx| v.goto(line, col, window, cx));
        }
        let focus = view.read(cx).focus_handle();
        let slot = self.panels.entry(project).or_default();
        slot.view = Some(view);
        slot._sub = Some(sub);
        if self.workspace.active_project().map(|p| p.id) == Some(project) {
            window.focus(&focus, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::panel_width;
    use gpui_kit::px;

    #[test]
    fn panel_target_width_lerps_between_panel_and_full() {
        use super::panel_target_width;
        assert_eq!(panel_target_width(1.0, px(1200.)), px(1200.));
        assert_eq!(panel_target_width(0.0, px(1200.)), px(540.));
        assert_eq!(panel_target_width(0.5, px(1200.)), px(870.));
    }

    #[test]
    fn panel_width_rules() {
        assert_eq!(panel_width(px(2000.)), px(900.));
        assert_eq!(panel_width(px(1200.)), px(540.));
        assert_eq!(panel_width(px(800.)), px(420.));
        assert_eq!(panel_width(px(500.)), px(500.));
    }
}

/// 計画5の 1 ファイルの動きを保つ間のつなぎ（Task 7 でエディタ列に置き換える）。
fn set_single_file(ws: &mut fude_core::workspace::Workspace, project: ProjectId, rel: Option<String>) {
    if let Some(tabs) = ws.editor_mut(project) {
        *tabs = fude_core::editor_tabs::EditorTabs::default();
        if let Some(rel) = rel {
            tabs.open(fude_core::editor_tabs::EditorItem { rel, mode: fude_core::panel::FileMode::Full });
        }
    }
}
