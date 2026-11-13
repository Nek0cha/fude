//! エディタ列（設計書 §4.1.3）。プロジェクトごとに、ファイルのタブと、タブごとの FileView を持つ。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fude_core::editor_tabs::{EditorItem, Opened};
use fude_core::panel::{FileMode, renamed_rel};
use fude_core::workspace::ProjectId;
use fude_fs::ProjectFs;
use gpui_kit::*;

use crate::dialog::{ButtonRole, Dialog, DialogButton, error_text};
use crate::columns::{Place, TREE_W, columns};
use crate::file_view::{FileView, FileViewEvent, initial_mode};
use crate::main_area::ColumnEl;
use crate::workspace_view::WorkspaceView;
use crate::wsl_host::project_fs;

/// エディタ列の出入りと、ターミナルの有無によるエディタの移動の動き（選択中のプロジェクトのもの）。
pub(crate) struct EditorAnim {
    pub presence: crate::anim::Presence,
    /// 1.0 = ターミナルが 0 枚の幅、0.0 = ターミナルと並べた幅。
    pub share: fude_core::motion::Fade,
    pub project: Option<ProjectId>,
    /// 閉じて、消える動きの間だけ描き続けるビュー。
    pub closing: Option<Entity<FileView>>,
}

impl Default for EditorAnim {
    fn default() -> Self {
        EditorAnim {
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
    /// 作り終えたらフォーカスを当てるもの（人が開いた・選んだもの）。描画が裏で作るものは入れない。
    focus_on_install: HashSet<String>,
    /// ProjectFs を用意できなかったもの。開き直す・選び直すまで作り直さない（描画のたびに試さない）。
    failed: HashSet<String>,
}

impl EditorSlot {
    pub(crate) fn request_focus(&mut self, rel: &str) {
        self.focus_on_install.insert(rel.to_string());
    }

    /// rel を作り終えたときにフォーカスを当てるか。一度使ったら消す。
    pub(crate) fn take_focus_request(&mut self, rel: &str) -> bool {
        self.focus_on_install.remove(rel)
    }

    pub(crate) fn mark_failed(&mut self, rel: &str) {
        self.failed.insert(rel.to_string());
    }

    pub(crate) fn clear_failed(&mut self, rel: &str) {
        self.failed.remove(rel);
    }

    /// rel のビューを作り始めてよいか（用意している途中でも、失敗したままでもない）。
    pub(crate) fn can_create(&self, rel: &str) -> bool {
        !self.loading.contains(rel) && !self.failed.contains(rel)
    }

    pub(crate) fn forget(&mut self, rel: &str) -> Option<Entity<FileView>> {
        self.focus_on_install.remove(rel);
        self.failed.remove(rel);
        self.goto.remove(rel);
        self.subs.remove(rel);
        self.loading.remove(rel);
        self.auto_mode.remove(rel);
        self.views.remove(rel)
    }

    /// 名前の変更に、表のキーを付いていかせる。
    pub(crate) fn rename(&mut self, from: &str, to: &str) {
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
        let rename_set = |set: &HashSet<String>| set.iter().map(|k| renamed_rel(k, from, to).unwrap_or_else(|| k.clone())).collect();
        self.auto_mode = rename_set(&self.auto_mode);
        self.focus_on_install = rename_set(&self.focus_on_install);
        // 失敗したものは、新しい名前で試し直させる。
        self.failed.retain(|k| renamed_rel(k, from, to).is_none());
        // 用意している途中のものは捨て、新しい名前で作り直させる（選んだときに作る）。
        self.loading.retain(|k| renamed_rel(k, from, to).is_none());
    }
}

/// 相対パスと未保存かの組から、閉じる前に確かめるもの（未保存のもの）を返す。選んでいないタブも含む。
pub(crate) fn unsaved_views(views: &[(String, bool)]) -> Vec<String> {
    views.iter().filter(|(_, dirty)| *dirty).map(|(rel, _)| rel.clone()).collect()
}

/// 監視で変わったと届いたもの（changed）のうち、開いているビュー（open）のもの。選んでいないタブも含む。
pub(crate) fn watched_views(changed: &[&str], open: &[&str]) -> Vec<String> {
    open.iter().filter(|rel| changed.contains(rel)).map(|rel| rel.to_string()).collect()
}

/// 名前を dest に変えると、未保存の変更（dirty の相対パス）を持つタブを置き換えてしまうなら、そのパス。
/// 置き換えたタブの変更を保存すると、名前を変えたファイルの中身を潰すため、名前の変更を止める。
pub(crate) fn rename_blocked(dest: &str, dirty: &[&str]) -> Option<String> {
    dirty.iter().find(|d| renamed_rel(d, dest, dest).is_some()).map(|d| d.to_string())
}

/// タブを閉じたあとのフォーカス。
#[derive(Debug, PartialEq)]
pub(crate) enum AfterClose {
    /// 今のまま（ターミナルなどにあるフォーカスを動かさない）。
    Keep,
    /// エディタ列の、新しく選ばれたタブへ。
    ShowActive,
    /// エディタ列が空になった。もう一方の側（ターミナル）へ。
    RestoreFocus,
}

pub(crate) fn focus_after_close(closed_was_active: bool, focus_in_editor: bool, now_empty: bool) -> AfterClose {
    if now_empty {
        AfterClose::RestoreFocus
    } else if closed_was_active || focus_in_editor {
        AfterClose::ShowActive
    } else {
        AfterClose::Keep
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
    pub(crate) fn observe_editor_column(&mut self, now: std::time::Instant) {
        use crate::theme::motion::{EDITOR_MOVE, PANEL_IN, PANEL_OUT, ease_out_quint};
        let project = self.workspace.active_project().map(|p| p.id);
        let has_tabs = self.workspace.active_project().is_some_and(|p| !p.tabs.is_empty());
        let open = self.editor_open();
        let reduced = self.reduced_motion;
        let target = if has_tabs { 0.0 } else { 1.0 };
        let a = &mut self.editor_anim;
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
    pub(crate) fn editor_animating(&self, now: std::time::Instant) -> bool {
        self.editor_anim.presence.is_animating(now) || self.editor_anim.share.is_animating(now)
    }

    /// メイン領域に描くツリーとエディタ列。main_w はメイン領域の幅。並べ方は columns で決め、
    /// 並べた列の幅には出入りの動きを掛ける（重ねた列は不透明度だけを変える）。
    pub(crate) fn render_columns(&mut self, main_w: Pixels, now: std::time::Instant, cx: &mut Context<Self>) -> (Option<ColumnEl>, Option<ColumnEl>) {
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return (None, None) };
        let main = f32::from(main_w);
        let tree_presence = self.tree_presence;
        let tree_shown = self.tree.open || !tree_presence.is_gone(now);
        let editor_presence = self.editor_anim.presence;
        let has_view = self.active_editor_view().is_some() || self.editor_anim.closing.is_some();
        let editor_shown = self.editor_open() || (has_view && !editor_presence.is_gone(now));
        let share = self.editor_anim.share.value(now);
        let with_terms = columns(main, true, tree_shown, editor_shown);
        let without = columns(main, false, tree_shown, editor_shown);
        // 置き方は、ターミナルの有無が変わり終えた側で決める。幅は動きの間を補う。
        let c = if share >= 1.0 { without } else { with_terms };
        let tree_w = match c.tree {
            Place::Inline => TREE_W * tree_presence.travel(now),
            _ => TREE_W,
        };
        let editor_target = {
            let fill = main - if c.tree == Place::Inline { tree_w } else { 0.0 };
            with_terms.editor_w + (fill - with_terms.editor_w) * share.clamp(0.0, 1.0)
        };
        let editor_w = match c.editor {
            Place::Inline => editor_target * editor_presence.travel(now),
            _ => editor_target,
        };
        let tree = if tree_shown {
            self.render_tree(cx, now).map(|el| ColumnEl {
                el: el.w(px(TREE_W)).h_full().into_any_element(),
                place: c.tree,
                w: px(tree_w),
                opacity: tree_presence.value(now),
            })
        } else {
            None
        };
        let editor = if editor_shown {
            let el = self.render_editor_column(project, px(editor_target), cx, now);
            Some(ColumnEl { el: el.into_any_element(), place: c.editor, w: px(editor_w), opacity: editor_presence.value(now) })
        } else {
            None
        };
        (tree, editor)
    }

    /// エディタ列の中身（タブ列と、選択中のタブの FileView）。中身の幅は w に固定し、出入りの動きで折り返し直さない。
    fn render_editor_column(&mut self, project: ProjectId, w: Pixels, cx: &mut Context<Self>, now: std::time::Instant) -> Div {
        let tabs = self.render_editor_tabs(project, cx, now);
        let view = self.active_editor_view().or_else(|| self.editor_anim.closing.clone());
        div()
            .w(w)
            .h_full()
            .flex()
            .flex_col()
            .child(tabs)
            .child(div().flex_1().min_h_0().children(view))
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
                slot.clear_failed(&rel);
                slot.request_focus(&rel);
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
            None => {
                let slot = self.editors.entry(project).or_default();
                slot.clear_failed(&rel);
                slot.request_focus(&rel);
                self.create_view(project, rel, window, cx);
            }
        }
    }

    /// 描画のたびに呼ぶ。選択中のプロジェクトの選択中のタブにビューが無ければ作る
    /// （セッションから戻したタブと、プロジェクトを切り替えたとき。作るのは選ばれたタブだけ）。
    pub(crate) fn ensure_active_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.workspace.active_project() else { return };
        let Some(rel) = p.editor.active().map(|i| i.rel.clone()) else { return };
        let project = p.id;
        let slot = self.editors.get(&project);
        if slot.is_some_and(|s| s.views.contains_key(&rel) || !s.can_create(&rel)) {
            return;
        }
        self.create_view(project, rel, window, cx);
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
        let focus_in_editor = self.project_views(project).iter().any(|v| v.read(cx).focus_handle().contains_focused(window, cx));
        let close = move |this: &mut WorkspaceView, window: &mut Window, cx: &mut Context<WorkspaceView>| {
            // 確かめている間に並びが変わっていてもよいよう、位置は相対パスで引き直す。
            let Some(tabs) = this.workspace.editor_mut(project) else { return };
            let Some(index) = tabs.position(&rel) else { return };
            let was_active = tabs.active_index() == Some(index);
            tabs.close(index);
            let now_empty = tabs.is_empty();
            let view = this.editors.get_mut(&project).and_then(|s| s.forget(&rel));
            match focus_after_close(was_active, focus_in_editor, now_empty) {
                AfterClose::RestoreFocus => {
                    // 消える動きの間だけ描き続ける。
                    this.editor_anim.closing = view;
                    this.restore_focus(window, cx);
                }
                AfterClose::ShowActive => this.show_active_tab(project, window, cx),
                AfterClose::Keep => {}
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
            let result = task.await;
            cx.update_window(window_handle, |_, window, cx| {
                this.update(cx, |this, cx| match result {
                    Ok(fs) => this.install_view(project, rel, fs, window, cx),
                    Err(e) => {
                        let Some(slot) = this.editors.get_mut(&project) else { return };
                        // 待っている間にタブを閉じたか名前が変わったなら、知らせない。
                        if !slot.loading.remove(&rel) {
                            return;
                        }
                        slot.mark_failed(&rel);
                        slot.focus_on_install.remove(&rel);
                        this.show_error(error_text("ファイルを開けません", &rel, &e), window, cx);
                    }
                })
                .ok();
            })
            .ok();
        })
        .detach();
    }

    fn install_view(&mut self, project: ProjectId, rel: String, fs: Arc<dyn ProjectFs>, window: &mut Window, cx: &mut Context<Self>) {
        // 待っている間に、プロジェクトかタブを閉じたか、名前が変わった。
        let Some(slot) = self.editors.get_mut(&project) else { return };
        if !slot.loading.remove(&rel) {
            return;
        }
        let auto = slot.auto_mode.remove(&rel);
        let goto = slot.goto.remove(&rel);
        let want_focus = slot.take_focus_request(&rel);
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
        // 描画が裏で作ったもの（プロジェクトを切り替えたとき）は、ターミナルからフォーカスを奪わない。
        if is_active && want_focus {
            window.focus(&focus, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AfterClose, AfterOpen, EditorSlot, Opened, after_open, focus_after_close, rename_blocked, unsaved_views, watched_views,
    };

    #[test]
    fn focus_request_is_consumed_and_follows_rename() {
        let mut slot = EditorSlot::default();
        slot.request_focus("a.rs");
        slot.rename("a.rs", "b.rs");
        assert!(!slot.take_focus_request("a.rs"));
        assert!(slot.take_focus_request("b.rs"));
        // 一度使ったら残らない（描画が作ったビューはフォーカスを取らない）。
        assert!(!slot.take_focus_request("b.rs"));
    }

    #[test]
    fn failed_tab_is_not_retried_until_cleared() {
        let mut slot = EditorSlot::default();
        slot.mark_failed("a.rs");
        assert!(!slot.can_create("a.rs"));
        // 開き直す・選び直すと、もう一度試す。
        slot.clear_failed("a.rs");
        assert!(slot.can_create("a.rs"));
        slot.forget("a.rs");
        assert!(slot.can_create("a.rs"));
    }

    #[test]
    fn closing_background_tab_keeps_focus() {
        // 選んでいないタブを閉じても、フォーカス（ターミナルなど）は動かさない。
        assert_eq!(focus_after_close(false, false, false), AfterClose::Keep);
        // 選択中のタブを閉じたら隣のタブへ。エディタ列にフォーカスがあったときも同じ。
        assert_eq!(focus_after_close(true, false, false), AfterClose::ShowActive);
        assert_eq!(focus_after_close(false, true, false), AfterClose::ShowActive);
        // 最後のタブを閉じたら、もう一方の側（ターミナル）へ。
        assert_eq!(focus_after_close(true, true, true), AfterClose::RestoreFocus);
        assert_eq!(focus_after_close(false, false, true), AfterClose::RestoreFocus);
    }

    #[test]
    fn rename_onto_unsaved_tab_is_blocked() {
        let dirty = ["b.rs", "src/x/mod.rs"];
        assert_eq!(rename_blocked("b.rs", &dirty).as_deref(), Some("b.rs"));
        assert_eq!(rename_blocked("src/x", &dirty).as_deref(), Some("src/x/mod.rs"));
        assert_eq!(rename_blocked("c.rs", &dirty), None);
        assert_eq!(rename_blocked("src/xy", &dirty), None);
    }

    #[test]
    fn unsaved_includes_inactive_tabs() {
        let views = [("a.rs".to_string(), false), ("b.rs".to_string(), true), ("c.rs".to_string(), true)];
        assert_eq!(unsaved_views(&views), ["b.rs", "c.rs"]);
    }

    #[test]
    fn watch_reaches_inactive_tabs() {
        assert_eq!(watched_views(&["b.rs", "x.rs"], &["a.rs", "b.rs"]), ["b.rs"]);
    }

    #[test]
    fn reopening_existing_tab_reuses_view() {
        assert_eq!(after_open(Opened::Existing(0), true, false, true), AfterOpen::Reuse { goto: true, set_mode: false });
        // 戻したタブ（ビューがまだ無い）は作る。
        assert_eq!(after_open(Opened::Existing(0), false, false, false), AfterOpen::Create);
        assert_eq!(after_open(Opened::New(1), false, true, false), AfterOpen::Create);
    }
}
