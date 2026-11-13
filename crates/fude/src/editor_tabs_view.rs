//! エディタ列のタブ列（設計書 §4.1.3）。ファイルのアイコン、名前、未保存の ●、ホバーで出る × を並べる。

use std::time::Instant;

use fude_core::workspace::ProjectId;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::file_icons::{file_icon, icon_for_file, tab_labels};
use crate::hover::HoverKey;
use crate::layout_anim::{tab_close_progress, tab_open_progress, tab_width_estimate};
use crate::theme::{Theme, size};
use crate::workspace_view::WorkspaceView;

/// タブ列の高さ。
pub(crate) const EDITOR_TABS_H: Pixels = px(36.);

/// 1 枚のタブの描き方。
struct TabLook {
    index: usize,
    name: String,
    parent: Option<String>,
    selected: bool,
    dirty: bool,
}

impl WorkspaceView {
    /// project のエディタ列のタブ列。収まらなければ横にスクロールし、選んだタブが見えるように寄せる。
    #[allow(dead_code)] // Task 9 の並べ方が使う。
    pub(crate) fn render_editor_tabs(&mut self, project: ProjectId, cx: &mut Context<Self>, now: Instant) -> Stateful<Div> {
        let theme = *cx.global::<Theme>();
        let Some(p) = self.workspace.projects().iter().find(|p| p.id == project) else { return div().id("editor-tabs") };
        let rels: Vec<String> = p.editor.items().iter().map(|i| i.rel.clone()).collect();
        let active = p.editor.active_index();
        // 選んだタブが変わったら、見えるところへ寄せる。
        if let Some(i) = active
            && self.editor_tab_scroll.1 != Some((project, i))
        {
            self.editor_tab_scroll.0.scroll_to_item(i);
            self.editor_tab_scroll.1 = Some((project, i));
        }
        let slot = self.editors.get(&project);
        let labels = tab_labels(&rels.iter().map(String::as_str).collect::<Vec<_>>());
        let mut tabs: Vec<AnyElement> = labels
            .into_iter()
            .enumerate()
            .map(|(index, (name, parent))| {
                let dirty = slot.and_then(|s| s.views.get(&rels[index])).is_some_and(|v| v.read(cx).is_dirty());
                let width = tab_width_estimate(&name);
                let el = self.render_editor_tab(project, TabLook { index, name, parent, selected: active == Some(index), dirty }, cx, now);
                // 出てきたタブは幅を広げながら、文字をフェードインする（ターミナルのタブと同じ。設計書 §6.3）。
                match self.anims.editor_opened.get(&rels[index]) {
                    Some(start) => {
                        let f = tab_open_progress(now.saturating_duration_since(*start), self.reduced_motion);
                        div().max_w(px(width * f)).overflow_hidden().flex_shrink_0().opacity(f).child(el).into_any_element()
                    }
                    None => el.into_any_element(),
                }
            })
            .collect();
        // 閉じたタブは、文字を消してから幅を縮める。
        for c in &self.anims.editor_closing {
            let (text, width, _) = tab_close_progress(now.saturating_duration_since(c.started), self.reduced_motion);
            let name = c.title.rsplit('/').next().unwrap_or_default().to_string();
            let ghost = div()
                .h(size::TAB_H)
                .w(px(tab_width_estimate(&name) * width))
                .flex_shrink_0()
                .overflow_hidden()
                .px(px(10.))
                .flex()
                .items_center()
                .whitespace_nowrap()
                .text_size(px(13.))
                .text_color(Rgba { a: theme.fg_dim.a * text, ..theme.fg_dim })
                .child(name)
                .into_any_element();
            tabs.insert(c.index.min(tabs.len()), ghost);
        }
        div()
            .id("editor-tabs")
            .h(EDITOR_TABS_H)
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .overflow_x_scroll()
            .track_scroll(&self.editor_tab_scroll.0)
            .children(tabs)
    }

    fn render_editor_tab(&self, project: ProjectId, look: TabLook, cx: &mut Context<Self>, now: Instant) -> Stateful<Div> {
        let theme = *cx.global::<Theme>();
        let index = look.index;
        let key = HoverKey::EditorTab(index);
        let close_key = HoverKey::EditorTabClose(index);
        let hovered = self.hover.value(key, now) > 0.0 || self.hover.value(close_key, now) > 0.0;
        let bg = if look.selected { theme.bg_hover } else { self.hover_bg(key, theme.bg_hover, now) };
        let fg = if look.selected { theme.fg_default } else { theme.fg_dim };
        // 未保存なら × の場所に ● を出し、ホバーで × に替える。
        let show_dot = look.dirty && !hovered;
        let close_color = if look.selected || hovered { theme.fg_dim } else { Rgba { a: 0.0, ..theme.fg_dim } };
        let close = self
            .hoverable(div().id(ElementId::Name(format!("editor-tab-close-{index}").into())), close_key, cx)
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .rounded(px(4.))
            .bg(self.hover_bg(close_key, theme.bg_hover, now))
            .map(|d| {
                if show_dot {
                    d.child(div().size(px(8.)).rounded_full().bg(fg))
                } else {
                    d.child(svg().path(IconName::X.path()).size(px(12.)).text_color(close_color))
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.close_editor_tab(project, index, window, cx);
            }));
        let icon = file_icon(icon_for_file(&look.name));
        self.hoverable(div().id(ElementId::Name(format!("editor-tab-{index}").into())), key, cx)
            .h(size::TAB_H)
            .max_w(size::TAB_MAX_W)
            .flex_shrink_0()
            .pl(px(10.))
            .pr(px(6.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .rounded(size::RADIUS_SM)
            .bg(bg)
            .text_color(fg)
            .text_size(px(13.))
            .child(icon)
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(look.name))
            .when_some(look.parent, |d, parent| {
                d.child(div().flex_shrink_0().whitespace_nowrap().text_size(px(11.)).text_color(theme.fg_dim).child(parent))
            })
            .child(close)
            .on_click(cx.listener(move |this, _, window, cx| this.select_editor_tab(project, index, window, cx)))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.close_editor_tab(project, index, window, cx);
                }),
            )
    }
}
