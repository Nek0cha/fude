//! タイトル行。ターミナルのタブ列、プロジェクトのピル、Windows のウィンドウ操作ボタンを置き、
//! タイトルバーを兼ねる（設計書 §4.1）。

use std::time::Instant;

use fude_core::workspace::Tab;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::hover::HoverKey;
use crate::theme::{Theme, size};
use crate::workspace_view::WorkspaceView;

const ICON: Pixels = px(14.);

pub(crate) fn icon(name: IconName, color: Rgba) -> Svg {
    svg().path(name.path()).size(ICON).flex_shrink_0().text_color(color)
}

impl WorkspaceView {
    pub(crate) fn render_title_row(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Div {
        let tabs: Vec<AnyElement> = match self.workspace.active_project() {
            Some(p) => {
                let active = p.active_tab;
                p.tabs.iter().map(|t| self.render_tab(t, Some(t.id) == active, cx, now).into_any_element()).collect()
            }
            None => Vec::new(),
        };
        let add_tab = self
            .tool_button("add-tab", HoverKey::AddTab, IconName::Plus, cx, now)
            .on_click(cx.listener(|this, _, _, cx| {
                this.new_default_tab();
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| this.open_shell_menu(ev.position, window, cx)),
            );
        let split = self
            .tool_button("split", HoverKey::Split, IconName::Columns2, cx, now)
            .on_click(cx.listener(|this, _, _, cx| {
                this.workspace.split_active_tab();
                cx.notify();
            }));

        // プロジェクトが無い（ホーム画面）ときは、タブ、＋、分割、ピルを出さない。タブが 0 枚なら分割も出さない。
        let has_project = self.workspace.active_project().is_some();
        let has_tabs = self.workspace.active_project().is_some_and(|p| !p.tabs.is_empty());
        div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(size::TITLE_ROW_H)
            .child(self.drag_area("title-lead", cx).w(size::TITLE_LEAD_W).h_full().flex_shrink_0())
            // タブが入りきらないときは、タブの列だけを縮めて右端を隠す。
            // ＋、分割、ピル、ウィンドウ操作ボタンは常に見えるようにする。
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .flex_shrink(1.)
                    .min_w_0()
                    .overflow_hidden()
                    .children(tabs),
            )
            .when(has_project, |row| {
                row.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.))
                        .pl(px(4.))
                        .flex_shrink_0()
                        .child(add_tab)
                        .when(has_tabs, |row| row.child(split)),
                )
            })
            .child(self.drag_area("title-gap", cx).flex_1().h_full().min_w(px(24.)))
            .child(self.render_pill(cx))
            .child(div().w(px(8.)).h_full().flex_shrink_0())
            .when(cfg!(windows), |row| row.child(self.render_caption_buttons(window, cx, now)))
            .when(!cfg!(windows), |row| row.child(div().w(px(8.)).flex_shrink_0()))
    }

    /// ウィンドウを掴んで動かせる空き領域。
    fn drag_area(&self, id: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        let el = div().id(id).window_control_area(WindowControlArea::Drag);
        if !cfg!(target_os = "macos") {
            return el;
        }
        // macOS ではアプリ側でドラッグとダブルクリックを処理する（app_owns_titlebar_drag）。
        el.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, _| this.should_move = true),
        )
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.should_move = false))
        .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, window, _| {
            if this.should_move {
                this.should_move = false;
                window.start_window_move();
            }
        }))
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                window.titlebar_double_click();
            }
        })
    }

    fn render_tab(&self, tab: &Tab, selected: bool, cx: &mut Context<Self>, now: Instant) -> Stateful<Div> {
        let theme = *cx.global::<Theme>();
        let id = tab.id;
        let key = HoverKey::Tab(id);
        let hovered = self.hover.value(key, now) > 0.0 || self.hover.value(HoverKey::TabClose(id), now) > 0.0;
        let bg = if selected { theme.bg_hover } else { self.hover_bg(key, theme.bg_hover, now) };
        let fg = if selected { theme.fg_default } else { theme.fg_dim };
        let close_bg = self.hover_bg(HoverKey::TabClose(id), theme.bg_hover, now);
        let close_color = if selected || hovered { theme.fg_dim } else { Rgba { a: 0.0, ..theme.fg_dim } };

        let close = self
            .hoverable(div().id(ElementId::Name(format!("tab-close-{id:?}").into())), HoverKey::TabClose(id), cx)
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .rounded(px(4.))
            .bg(close_bg)
            .child(svg().path(IconName::X.path()).size(px(12.)).text_color(close_color))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.workspace.close_tab(id);
                cx.notify();
            }));

        self.hoverable(div().id(ElementId::Name(format!("tab-{id:?}").into())), key, cx)
            .h(size::TAB_H)
            .max_w(size::TAB_MAX_W)
            .flex_shrink_0()
            .px(px(10.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .rounded(size::RADIUS_SM)
            .bg(bg)
            .text_color(fg)
            .text_size(px(13.))
            .child(icon(IconName::Terminal, fg))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(tab.title()))
            .child(close)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.workspace.activate_tab(id);
                cx.notify();
            }))
    }

    fn tool_button(
        &self,
        id: &'static str,
        key: HoverKey,
        name: IconName,
        cx: &mut Context<Self>,
        now: Instant,
    ) -> Stateful<Div> {
        let theme = *cx.global::<Theme>();
        let bg = self.hover_bg(key, theme.bg_hover, now);
        let fg = theme.fg_dim;
        self.hoverable(div().id(id), key, cx)
            .size(size::TAB_H)
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .rounded(size::RADIUS_SM)
            .bg(bg)
            .child(icon(name, fg))
    }

    /// プロジェクト名とブランチ名。この計画では表示だけで、押しても何も起きない。
    fn render_pill(&self, cx: &mut Context<Self>) -> Div {
        let theme = *cx.global::<Theme>();
        let Some(project) = self.workspace.active_project() else { return div() };
        let branch = self.branches.get(&project.id).cloned().flatten();
        div()
            .h(px(24.))
            .px(px(10.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .flex_shrink_0()
            .rounded(px(12.))
            .bg(theme.bg_hover)
            .text_size(px(12.))
            .child(div().text_color(theme.fg_default).child(project.name.clone()))
            .when_some(branch, |pill, branch| {
                pill.child(icon(IconName::GitBranch, theme.fg_dim).size(px(12.)))
                    .child(div().text_color(theme.fg_dim).child(branch))
            })
    }

    /// Windows の最小化、最大化、閉じるボタン。クリックは OS が処理する（WindowControlArea）。
    fn render_caption_buttons(&self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Div {
        let theme = *cx.global::<Theme>();
        let max_icon = if window.is_maximized() { IconName::WindowRestore } else { IconName::WindowMaximize };
        let buttons = [
            ("caption-min", HoverKey::CaptionMin, WindowControlArea::Min, IconName::WindowMinimize, theme.bg_hover),
            ("caption-max", HoverKey::CaptionMax, WindowControlArea::Max, max_icon, theme.bg_hover),
            ("caption-close", HoverKey::CaptionClose, WindowControlArea::Close, IconName::WindowClose, theme.window_close_hover),
        ];
        let fg = theme.fg_default;
        div().flex().flex_row().h_full().flex_shrink_0().children(buttons.map(|(id, key, area, name, hover)| {
            self.hoverable(div().id(id), key, cx)
                .window_control_area(area)
                .w(size::CAPTION_BTN_W)
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(self.hover_bg(key, hover, now))
                .child(icon(name, fg))
        }))
    }
}
