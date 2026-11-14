//! ピン列（設計書 §4.1、§4.1.3）。Markdown を 1 つだけ右端に留め、プレビューで出す。プロジェクトごとに持つ。

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::Instant;

use fude_core::text::decode_text;
use fude_core::workspace::{ProjectId, ProjectRoot};
use fude_fs::{ProjectFs, RelPath};
use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::md_preview::MdPreview;
use crate::theme::{Theme, size, ui_font};
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

/// ピンの切り替え。同じものなら外し、別のものなら入れ替える。
pub(crate) fn pin_after_toggle(current: Option<&str>, rel: &str) -> Option<String> {
    if current == Some(rel) { None } else { Some(rel.to_string()) }
}

pub enum PinViewEvent {
    /// エディタ列のタブで開く。
    OpenInEditor(String),
    Unpin,
}

pub struct PinView {
    fs: Arc<dyn ProjectFs>,
    root: ProjectRoot,
    rel: String,
    preview: Option<MdPreview>,
    missing: bool,
}

impl EventEmitter<PinViewEvent> for PinView {}

impl PinView {
    pub fn new(fs: Arc<dyn ProjectFs>, root: ProjectRoot, rel: String, cx: &mut Context<Self>) -> Self {
        let mut this = PinView { fs, root, rel, preview: None, missing: false };
        this.reload(cx);
        this
    }

    pub fn rel(&self) -> &str {
        &self.rel
    }

    /// 留めるファイルが替わった（入れ替え、名前の変更）。読み直す。
    pub fn set_rel(&mut self, rel: String, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.preview {
            p.set_md_rel(rel.clone());
        }
        self.rel = rel;
        self.reload(cx);
    }

    /// 保存された中身を裏で読み直す。
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let fs = self.fs.clone();
        let rel = RelPath::new(&self.rel).map_err(io::Error::from);
        let task = cx.background_spawn(async move { fs.read(&rel?) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.loaded(result, cx)).ok();
        })
        .detach();
    }

    fn loaded(&mut self, result: io::Result<Vec<u8>>, cx: &mut Context<Self>) {
        let text = match result.ok().and_then(|b| decode_text(&b).ok()) {
            Some(doc) => doc.text,
            None => {
                self.missing = true;
                cx.notify();
                return;
            }
        };
        self.missing = false;
        match &self.preview {
            Some(p) => p.set_text(&text, cx),
            None => self.preview = Some(MdPreview::new(&text, self.root.clone(), self.rel.clone(), cx)),
        }
        cx.notify();
    }
}

impl Render for PinView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let name = self.rel.rsplit('/').next().unwrap_or(&self.rel).to_string();
        let parent = self.rel.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        let button = |id: &'static str, name: IconName| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(size::RADIUS_SM)
                .hover(|s| s.bg(theme.bg_hover))
                .child(icon(name, theme.fg_dim))
        };
        let header = div()
            .h(px(36.))
            .px(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(icon(IconName::Pin, theme.accent))
            .child(div().flex_shrink_0().text_color(theme.fg_default).child(name))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_color(theme.fg_dim).child(parent))
            .child(button("pin-open", IconName::File).on_click(cx.listener(|this, _, _, cx| {
                cx.emit(PinViewEvent::OpenInEditor(this.rel.clone()));
            })))
            .child(button("pin-unpin", IconName::X).on_click(cx.listener(|_, _, _, cx| cx.emit(PinViewEvent::Unpin))));
        let body: AnyElement = match (&self.preview, self.missing) {
            (_, true) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_dim)
                .child("ファイルが見つかりません")
                .into_any_element(),
            (Some(p), false) => {
                let this = cx.entity().downgrade();
                let view = p.element(
                    theme.accent.into(),
                    move |rel, _, cx| {
                        this.update(cx, |_, cx| cx.emit(PinViewEvent::OpenInEditor(rel))).ok();
                    },
                    cx,
                );
                div().size_full().pl(px(16.)).pr(px(8.)).pt(px(8.)).child(view).into_any_element()
            }
            (None, false) => div().size_full().into_any_element(),
        };
        div()
            .id("pin-view")
            .size_full()
            .flex()
            .flex_col()
            .font(ui_font())
            .text_size(px(13.))
            .child(header)
            .child(div().flex_1().min_h_0().text_size(px(14.)).child(body))
    }
}

/// ピン列のビューと出入りの動き（選択中のプロジェクトのもの）。
pub(crate) struct PinSlot {
    pub views: HashMap<ProjectId, (Entity<PinView>, Subscription)>,
    pub presence: crate::anim::Presence,
    pub project: Option<ProjectId>,
    /// 外して、消える動きの間だけ描き続けるビュー。
    pub closing: Option<Entity<PinView>>,
}

impl Default for PinSlot {
    fn default() -> Self {
        PinSlot { views: HashMap::new(), presence: crate::anim::Presence::hidden(), project: None, closing: None }
    }
}

impl WorkspaceView {
    /// 描画のたびに呼ぶ。各プロジェクトのピンとビューを合わせ、出入りの動きを始める。
    pub(crate) fn sync_pins(&mut self, now: Instant, window: &mut Window, cx: &mut Context<Self>) {
        use crate::theme::motion::{PANEL_IN, PANEL_OUT};
        let pins: Vec<(ProjectId, ProjectRoot, Option<String>)> =
            self.workspace.projects().iter().map(|p| (p.id, p.root.clone(), p.pin.clone())).collect();
        let active = self.workspace.active_project().map(|p| p.id);
        // 閉じたプロジェクトのビューを捨てる。
        self.pins.views.retain(|id, _| pins.iter().any(|(p, _, _)| p == id));
        for (project, root, pin) in pins {
            match (pin, self.pins.views.get(&project).map(|(v, _)| v.clone())) {
                (Some(rel), Some(view)) => {
                    if view.read(cx).rel() != rel {
                        view.update(cx, |v, cx| v.set_rel(rel, cx));
                    }
                }
                (Some(rel), None) => {
                    let Some(fs) = self.live_fs(project) else { continue };
                    let view = cx.new(|cx| PinView::new(fs, root, rel, cx));
                    let sub = cx.subscribe_in(&view, window, move |this, _, ev: &PinViewEvent, window, cx| match ev {
                        PinViewEvent::OpenInEditor(rel) => this.open_in_editor(project, rel.clone(), None, None, window, cx),
                        PinViewEvent::Unpin => {
                            this.workspace.set_pin(project, None);
                            cx.notify();
                        }
                    });
                    self.pins.views.insert(project, (view, sub));
                }
                (None, Some(view)) => {
                    if Some(project) == active {
                        self.pins.closing = Some(view);
                    }
                    self.pins.views.remove(&project);
                }
                (None, None) => {}
            }
        }
        let open = active.is_some_and(|p| self.pins.views.contains_key(&p));
        let reduced = self.reduced_motion;
        let slot = &mut self.pins;
        if slot.project != active {
            // プロジェクトを切り替えたときは動かさない。
            slot.project = active;
            slot.presence = if open { crate::anim::Presence::shown() } else { crate::anim::Presence::hidden() };
            slot.closing = None;
            return;
        }
        if open && !slot.presence.visible() {
            slot.closing = None;
            slot.presence.show(PANEL_IN, reduced, now);
        } else if !open && slot.presence.visible() {
            slot.presence.hide(PANEL_OUT, reduced, now);
        }
        if slot.presence.is_gone(now) {
            slot.closing = None;
        }
    }

    /// 選択中のプロジェクトのピン列のビュー（外して消えていく間はそのビュー）。
    pub(crate) fn pin_view(&self) -> Option<Entity<PinView>> {
        let project = self.workspace.active_project()?.id;
        self.pins.views.get(&project).map(|(v, _)| v.clone()).or_else(|| self.pins.closing.clone())
    }

    /// ピン列を出しているか（消えていく間も含む）。
    pub(crate) fn pin_shown(&self, now: Instant) -> bool {
        self.pin_view().is_some() && !self.pins.presence.is_gone(now)
    }

    /// 選択中のエディタのタブ（Markdown）を留める、外す、入れ替える。rel が無ければピンを外す。
    pub(crate) fn toggle_pin(&mut self, project: ProjectId, rel: Option<String>, cx: &mut Context<Self>) {
        let Some(p) = self.workspace.projects().iter().find(|p| p.id == project) else { return };
        let next = match rel {
            Some(rel) => pin_after_toggle(p.pin.as_deref(), &rel),
            None => None,
        };
        self.workspace.set_pin(project, next);
        cx.notify();
    }

    /// 監視で変わったファイルにピンのファイルがあれば読み直す。
    pub(crate) fn reload_pin_if_changed(&mut self, project: ProjectId, changed: &[&str], cx: &mut Context<Self>) {
        let Some((view, _)) = self.pins.views.get(&project) else { return };
        let rel = view.read(cx).rel().to_string();
        if !crate::editor_column::watched_views(changed, &[rel.as_str()]).is_empty() {
            view.update(cx, |v, cx| v.reload(cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pin_after_toggle;

    #[test]
    fn toggle_pin_rules() {
        assert_eq!(pin_after_toggle(None, "a.md"), Some("a.md".to_string()));
        assert_eq!(pin_after_toggle(Some("a.md"), "a.md"), None);
        // 別の Markdown を留めると入れ替わる。
        assert_eq!(pin_after_toggle(Some("a.md"), "b.md"), Some("b.md".to_string()));
    }
}
