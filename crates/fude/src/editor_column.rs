//! エディタ列（設計書 §4.1.3）。プロジェクトごとに、ファイルのタブと、タブごとの FileView を持つ。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fude_core::editor_tabs::{EditorItem, Opened};
use fude_core::panel::{FileMode, renamed_rel};
use fude_core::workspace::ProjectId;
use fude_fs::ProjectFs;
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

/// プロジェクトごとのエディタ列の状態。キーはタブの相対パス。
#[derive(Default)]
pub(crate) struct EditorSlot {
    /// 作った FileView。選んでいないタブのものも捨てない（未保存の変更を持ち続けるため）。
    pub views: HashMap<String, Entity<FileView>>,
    /// ビューを作ったときに移る行と桁（1 始まり）。
    pub goto: HashMap<String, (u32, Option<u32>)>,
    subs: HashMap<String, Subscription>,
    /// ProjectFs を裏で用意している間のもの。
    loading: HashSet<String>,
    /// 作ったあと、Git の状態が届いたら表示の仕方を決め直すもの。
    auto_mode: HashSet<String>,
}

impl EditorSlot {
    fn forget(&mut self, rel: &str) -> Option<Entity<FileView>> {
        self.goto.remove(rel);
        self.subs.remove(rel);
        self.loading.remove(rel);
        self.auto_mode.remove(rel);
        self.views.remove(rel)
    }

    /// 名前の変更に、表のキーを付いていかせる。
    fn rename(&mut self, from: &str, to: &str) {
        fn rekey<V>(map: &mut HashMap<String, V>, from: &str, to: &str) {
            let keys: Vec<String> = map.keys().filter(|k| renamed_rel(k, from, to).is_some()).cloned().collect();
            for k in keys {
                if let (Some(v), Some(new)) = (map.remove(&k), renamed_rel(&k, from, to)) {
                    map.insert(new, v);
                }
            }
        }
        rekey(&mut self.views, from, to);
        rekey(&mut self.goto, from, to);
        rekey(&mut self.subs, from, to);
        self.auto_mode = self.auto_mode.iter().map(|k| renamed_rel(k, from, to).unwrap_or_else(|| k.clone())).collect();
        // 用意している途中のものは捨て、新しい名前で作り直させる（選んだときに作る）。
        self.loading.retain(|k| renamed_rel(k, from, to).is_none());
    }
}

/// 開いたあとにすること。
#[derive(Debug, PartialEq)]
pub(crate) enum AfterOpen {
    /// FileView を作る（新しいタブか、まだビューの無い戻したタブ）。
    Create,
    /// 今のビューを使う。goto なら行を移し、set_mode なら表示の仕方を変える。
    Reuse { goto: bool, set_mode: bool },
}

pub(crate) fn after_open(opened: Opened, has_view: bool, mode_given: bool, at_given: bool) -> AfterOpen {
    match opened {
        Opened::Existing(_) if has_view => AfterOpen::Reuse { goto: at_given, set_mode: mode_given },
        _ => AfterOpen::Create,
    }
}

impl WorkspaceView {
    /// 描画のたびに呼ぶ。エディタ列の出入りと、ターミナルの有無によるエディタの移動の動きを始める。
    pub(crate) fn observe_panel(&mut self, now: std::time::Instant) {
        use crate::theme::motion::{EDITOR_MOVE, PANEL_IN, PANEL_OUT, ease_out_quint};
        let project = self.workspace.active_project().map(|p| p.id);
        let has_tabs = self.workspace.active_project().is_some_and(|p| !p.tabs.is_empty());
        let open = self.editor_open();
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

    /// エディタ列とエディタの移動が動いている（ターミナルの大きさが変わる）。
    pub(crate) fn panel_animating(&self, now: std::time::Instant) -> bool {
        self.panel_anim.presence.is_animating(now) || self.panel_anim.share.is_animating(now)
    }

    /// メイン領域に描くエディタ列のビュー、幅、不透明度。
    pub(crate) fn panel_for_render(&self, main_w: Pixels, now: std::time::Instant) -> Option<(AnyView, Pixels, f32)> {
        let view = self.active_editor_view().or_else(|| self.panel_anim.closing.clone())?;
        let p = &self.panel_anim.presence;
        let w = panel_target_width(self.panel_anim.share.value(now), main_w) * p.travel(now);
        Some((AnyView::from(view), w, p.value(now)))
    }

    /// 選択中のプロジェクトのエディタ列にタブがあるか。
    pub(crate) fn editor_open(&self) -> bool {
        self.workspace.active_project().is_some_and(|p| !p.editor.is_empty())
    }

    /// 選択中のプロジェクトの、選択中のタブの FileView（作っていれば）。
    pub(crate) fn active_editor_view(&self) -> Option<Entity<FileView>> {
        let p = self.workspace.active_project()?;
        let rel = &p.editor.active()?.rel;
        self.editors.get(&p.id)?.views.get(rel).cloned()
    }

    /// project で作った FileView の全部（選んでいないタブのものも含む）。
    pub(crate) fn project_views(&self, project: ProjectId) -> Vec<Entity<FileView>> {
        self.editors.get(&project).map(|s| s.views.values().cloned().collect()).unwrap_or_default()
    }

    /// project のエディタ列に rel を開く。同じファイルのタブがあれば、それを選ぶだけにする（未保存の変更は残す）。
    /// mode が None なら Git の状態から決める（既存のタブなら変えない）。at は移る行と桁（1 始まり）。
    pub(crate) fn open_in_editor(
        &mut self,
        project: ProjectId,
        rel: String,
        mode: Option<FileMode>,
        at: Option<(u32, Option<u32>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mark = self.git_mark(project, &rel);
        let auto = mode.is_none() && self.status_pending(project);
        let Some(tabs) = self.workspace.editor_mut(project) else { return };
        let opened = tabs.open(EditorItem { rel: rel.clone(), mode: mode.unwrap_or_else(|| initial_mode(mark)) });
        if let (Opened::Existing(i), Some(mode)) = (opened, mode) {
            tabs.set_mode(i, mode);
        }
        let slot = self.editors.entry(project).or_default();
        let view = slot.views.get(&rel).cloned();
        match (after_open(opened, view.is_some(), mode.is_some(), at.is_some()), view) {
            (AfterOpen::Reuse { goto, set_mode }, Some(view)) => {
                if let (true, Some(mode)) = (set_mode, mode) {
                    view.update(cx, |v, cx| v.set_mode(mode, cx));
                }
                if let (true, Some((line, col))) = (goto, at) {
                    view.update(cx, |v, cx| v.goto(line, col, window, cx));
                }
                window.focus(&view.read(cx).focus_handle(), cx);
            }
            _ => {
                if let Some(at) = at {
                    slot.goto.insert(rel.clone(), at);
                }
                if auto {
                    slot.auto_mode.insert(rel.clone());
                }
                self.create_view(project, rel, window, cx);
            }
        }
        cx.notify();
    }

    /// project のエディタ列の index のタブを選ぶ。ビューがまだ無ければ作る。
    #[allow(dead_code)] // Task 8 のタブ列が使う。
    pub(crate) fn select_editor_tab(&mut self, project: ProjectId, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tabs) = self.workspace.editor_mut(project) else { return };
        tabs.select(index);
        self.show_active_tab(project, window, cx);
        cx.notify();
    }

    /// project の選択中のタブを出す。ビューがあればフォーカスを当て、無ければ作る（作り終えたらフォーカスが移る）。
    pub(crate) fn show_active_tab(&mut self, project: ProjectId, window: &mut Window, cx: &mut Context<Self>) {
        let rel = self.workspace.projects().iter().find(|p| p.id == project).and_then(|p| p.editor.active()).map(|i| i.rel.clone());
        let Some(rel) = rel else { return };
        match self.editors.get(&project).and_then(|s| s.views.get(&rel)).cloned() {
            Some(view) => window.focus(&view.read(cx).focus_handle(), cx),
            None => self.create_view(project, rel, window, cx),
        }
    }

    /// project のエディタ列の index のタブを閉じる。未保存なら、保存・保存しない・キャンセルを聞く。
    pub(crate) fn close_editor_tab(&mut self, project: ProjectId, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let rel = self
            .workspace
            .projects()
            .iter()
            .find(|p| p.id == project)
            .and_then(|p| p.editor.items().get(index))
            .map(|i| i.rel.clone());
        let Some(rel) = rel else { return };
        let view = self.editors.get(&project).and_then(|s| s.views.get(&rel)).cloned();
        let close = move |this: &mut WorkspaceView, window: &mut Window, cx: &mut Context<WorkspaceView>| {
            // 確かめている間に並びが変わっていてもよいよう、位置は相対パスで引き直す。
            let Some(tabs) = this.workspace.editor_mut(project) else { return };
            let Some(index) = tabs.position(&rel) else { return };
            tabs.close(index);
            let now_empty = tabs.is_empty();
            let view = this.editors.get_mut(&project).and_then(|s| s.forget(&rel));
            if now_empty {
                // 消える動きの間だけ描き続ける。
                this.panel_anim.closing = view;
                this.restore_focus(window, cx);
            } else {
                this.show_active_tab(project, window, cx);
            }
            cx.notify();
        };
        match view.filter(|v| v.read(cx).is_dirty()) {
            Some(view) => self.confirm_discard(view, window, cx, close),
            None => close(self, window, cx),
        }
    }

    /// view に未保存の変更がある。保存・保存しない・キャンセルを聞き、キャンセル以外なら then を呼ぶ。
    fn confirm_discard(
        &mut self,
        view: Entity<FileView>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>) + 'static,
    ) {
        let name = view.read(cx).rel.rsplit('/').next().unwrap_or_default().to_string();
        let dialog = Dialog {
            message: format!("{name} の変更を保存しますか？"),
            buttons: vec![
                DialogButton { label: "保存", role: ButtonRole::Primary },
                DialogButton { label: "保存しない", role: ButtonRole::Danger },
                DialogButton { label: "キャンセル", role: ButtonRole::Cancel },
            ],
            on_choose: Box::new(move |i, this, window, cx| match i {
                0 => this.save_then(vec![view], window, cx, then),
                1 => then(this, window, cx),
                _ => {}
            }),
        };
        self.show_dialog(dialog, window, cx);
    }

    /// ツリーで from の名前が to に変わった。タブ、表のキー、FileView の相対パスを合わせる。
    pub(crate) fn editor_renamed(&mut self, project: ProjectId, from: &str, to: &str, cx: &mut Context<Self>) {
        if let Some(tabs) = self.workspace.editor_mut(project) {
            tabs.rename(from, to);
        }
        let Some(slot) = self.editors.get_mut(&project) else { return };
        slot.rename(from, to);
        for (rel, view) in &slot.views {
            if view.read(cx).rel != *rel {
                view.update(cx, |v, cx| v.set_rel(rel.clone(), cx));
            }
        }
        cx.notify();
    }

    /// エディタで Esc が親まで来た。最後に使ったターミナルにフォーカスを戻す（ターミナルが無ければ今のまま）。
    fn leave_editor(&mut self, cx: &mut Context<Self>) {
        if self.workspace.active_project().and_then(|p| p.active_tab()).is_some() {
            self.refocus_pane();
            cx.notify();
        }
    }

    /// rel の FileView を作る。ProjectFs を裏で用意してから作る（WSL は fude serve の起動を待つことがある）。
    fn create_view(&mut self, project: ProjectId, rel: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.projects().iter().find(|p| p.id == project).map(|p| p.root.clone()) else {
            return;
        };
        let slot = self.editors.entry(project).or_default();
        if !slot.loading.insert(rel.clone()) {
            return;
        }
        let hosts = self.wsl_hosts.clone();
        let task = cx.background_spawn(async move { project_fs(&root, &hosts) });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let fs: Arc<dyn ProjectFs> = match task.await {
                Ok(fs) => fs,
                Err(e) => {
                    eprintln!("ファイルを開けない: {rel}: {e}");
                    return;
                }
            };
            cx.update_window(window_handle, |_, window, cx| {
                this.update(cx, |this, cx| this.install_view(project, rel, fs, window, cx)).ok();
            })
            .ok();
        })
        .detach();
    }

    fn install_view(&mut self, project: ProjectId, rel: String, fs: Arc<dyn ProjectFs>, window: &mut Window, cx: &mut Context<Self>) {
        let slot = self.editors.entry(project).or_default();
        // 待っている間に、タブを閉じたか名前が変わった。
        if !slot.loading.remove(&rel) {
            return;
        }
        let auto = slot.auto_mode.remove(&rel);
        let goto = slot.goto.remove(&rel);
        let Some(p) = self.workspace.projects().iter().find(|p| p.id == project) else { return };
        let Some(item) = p.editor.position(&rel).map(|i| p.editor.items()[i].clone()) else { return };
        let is_active =
            p.editor.active().is_some_and(|a| a.rel == rel) && self.workspace.active_project().map(|a| a.id) == Some(project);
        let mark = self.git_mark(project, &rel);
        let view = cx.new(|cx| {
            let mut v = FileView::new(fs, rel.clone(), item.mode, mark, cx);
            v.set_auto_mode(auto);
            v
        });
        // 状態がすでに届いていれば、すぐに決め直す。
        if auto && !self.status_pending(project) {
            view.update(cx, |v, cx| v.set_mark(mark, cx));
        }
        let sub = cx.subscribe_in(&view, window, move |this, _, ev: &FileViewEvent, window, cx| match ev {
            FileViewEvent::LeaveEditor => this.leave_editor(cx),
            FileViewEvent::Dirty | FileViewEvent::Saved => cx.notify(),
            FileViewEvent::SaveFailed(message) => this.show_error(message.clone(), window, cx),
        });
        if let Some((line, col)) = goto {
            view.update(cx, |v, cx| v.goto(line, col, window, cx));
        }
        let focus = view.read(cx).focus_handle();
        let slot = self.editors.entry(project).or_default();
        slot.views.insert(rel.clone(), view);
        slot.subs.insert(rel, sub);
        if is_active {
            window.focus(&focus, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{AfterOpen, Opened, after_open, panel_target_width, panel_width};
    use gpui_kit::px;

    #[test]
    fn reopening_existing_tab_reuses_view() {
        assert_eq!(after_open(Opened::Existing(0), true, false, true), AfterOpen::Reuse { goto: true, set_mode: false });
        // 戻したタブ（ビューがまだ無い）は作る。
        assert_eq!(after_open(Opened::Existing(0), false, false, false), AfterOpen::Create);
        assert_eq!(after_open(Opened::New(1), false, true, false), AfterOpen::Create);
    }

    #[test]
    fn panel_target_width_lerps_between_panel_and_full() {
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
