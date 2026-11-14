//! プロジェクトレール（左端）。開いているプロジェクトを、頭文字と色のアイコンで縦に並べる。

use std::time::Instant;

use fude_core::workspace::{Project, project_color_index};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::hover::HoverKey;
use crate::theme::{Theme, size};
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

impl WorkspaceView {
    pub(crate) fn render_rail(&self, cx: &mut Context<Self>, now: Instant) -> Div {
        let active = self.workspace.active_project().map(|p| p.id);
        let icons: Vec<Stateful<Div>> = self
            .workspace
            .projects()
            .iter()
            .map(|p| self.render_project_icon(p, Some(p.id) == active, cx, now))
            .collect();

        let theme = *cx.global::<Theme>();
        let add = self
            .hoverable(div().id("add-project"), HoverKey::AddProject, cx)
            .size(size::PROJECT_ICON)
            .flex()
            .items_center()
            .justify_center()
            .rounded(size::RADIUS_MD)
            .bg(self.hover_bg(HoverKey::AddProject, theme.bg_hover, now))
            .child(icon(IconName::Plus, theme.fg_dim))
            .on_click(cx.listener(|this, _, _, cx| this.prompt_open_project(cx)));

        div()
            .w(size::RAIL_W)
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .pt(px(4.))
            .pb(px(12.))
            .children(icons)
            .child(div().flex_1())
            .child(add)
    }

    fn render_project_icon(&self, project: &Project, selected: bool, cx: &mut Context<Self>, now: Instant) -> Stateful<Div> {
        let theme = *cx.global::<Theme>();
        let id = project.id;
        let key = HoverKey::Project(id);
        let initial: String = project.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        let color = theme.project_colors[project_color_index(&project.root)];
        // 選択中の印は、アイコンの大きさを変えずにレールの左端へ縦線で出す。
        let marker = div()
            .absolute()
            .left(px(-8.))
            .top(px(6.))
            .w(px(3.))
            .h(px(20.))
            .rounded(px(2.))
            .when(selected, |m| m.bg(theme.accent));

        self.hoverable(div().id(ElementId::Name(format!("project-{id:?}").into())), key, cx)
            .relative()
            .size(size::PROJECT_ICON)
            .flex()
            .items_center()
            .justify_center()
            .rounded(size::RADIUS_MD)
            .bg(color)
            .child(
                div()
                    .absolute()
                    .size_full()
                    .rounded(size::RADIUS_MD)
                    .bg(self.hover_bg(key, theme.bg_hover, now)),
            )
            .child(marker)
            // 返事待ちの点（§5.8）。アイコンの大きさは変えない。
            .when(self.project_waiting(project), |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(-2.))
                        .right(px(-2.))
                        .size(px(8.))
                        .rounded_full()
                        .border_2()
                        .border_color(theme.bg_window)
                        .bg(theme.warn),
                )
            })
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg_default)
                    .child(initial),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| this.open_project_menu(id, ev.position, window, cx)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.workspace.activate_project(id);
                this.refresh_active_branch(cx);
                cx.notify();
            }))
    }
}
