//! ターミナルのタブを縦に並べる表示（設計書 §4.1.3）。路線図とメイン領域の間に列を出す。

use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::hover::HoverKey;
use crate::layout_anim::{tab_close_progress, tab_open_progress};
use crate::theme::Theme;
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

/// 縦のタブの列の幅。
pub(crate) const VTABS_W: f32 = 160.0;
/// 縦のタブの行の高さ。
pub(crate) const VTAB_H: f32 = 30.0;

/// タブの行の高さ。開いてから opened_ago、閉じてから closed_ago。横のタブの幅と同じ動きを高さに使う。
/// 動きを減らす設定では高さを変えない（文字の濃さだけが変わる）。
pub(crate) fn row_height(opened_ago: Option<Duration>, closed_ago: Option<Duration>, reduced: bool) -> f32 {
    if reduced {
        return VTAB_H;
    }
    if let Some(t) = closed_ago {
        return VTAB_H * tab_close_progress(t, false).1;
    }
    match opened_ago {
        Some(t) => VTAB_H * tab_open_progress(t, false),
        None => VTAB_H,
    }
}

impl WorkspaceView {
    /// パレットの「ターミナルのタブを縦に並べる」。すぐ保存される（schedule_save が値を書く）。
    pub(crate) fn toggle_vertical_tabs(&mut self, cx: &mut Context<Self>) {
        self.vertical_tabs = !self.vertical_tabs;
        cx.notify();
    }

    /// 描画のたびに呼ぶ。縦の列の幅の動きを始める（路線図と同じ値）。ホーム画面では出さない。
    pub(crate) fn observe_vertical_tabs(&mut self, window: &mut Window, now: Instant) {
        use crate::theme::motion::{PANEL_IN, PANEL_OUT, ease_out_quint};
        let shown = self.vertical_tabs && self.workspace.active_project().is_some();
        let target = if shown { VTABS_W } else { 0.0 };
        if self.vtabs_w.target() != target {
            let duration = match (self.reduced_motion, shown) {
                (true, _) => Duration::ZERO,
                (false, true) => PANEL_IN,
                (false, false) => PANEL_OUT,
            };
            self.vtabs_w.animate_to(target, duration, ease_out_quint, now);
        }
        if self.vtabs_w.is_animating(now) {
            window.request_animation_frame();
        }
    }

    pub(crate) fn vertical_tabs_width(&self, now: Instant) -> f32 {
        self.vtabs_w.value(now)
    }

    /// 縦のタブの列。幅が 0 なら None。中身の幅は VTABS_W に固定し、出入りでは切って見せる。
    pub(crate) fn render_vertical_tabs(&mut self, now: Instant, cx: &mut Context<Self>) -> Option<AnyElement> {
        let width = self.vtabs_w.value(now);
        if width < 0.5 {
            return None;
        }
        let theme = *cx.global::<Theme>();
        let reduced = self.reduced_motion;
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(p) = self.workspace.active_project() {
            let active = p.active_tab;
            for t in &p.tabs {
                let opened = self.anims.opened.get(&t.id).map(|s| now.saturating_duration_since(*s));
                let h = row_height(opened, None, reduced);
                let fade = opened.map_or(1.0, |d| tab_open_progress(d, reduced));
                let el = self.render_tab(t, Some(t.id) == active, true, cx, now);
                rows.push(div().h(px(h)).overflow_hidden().flex_shrink_0().opacity(fade).child(el).into_any_element());
            }
        }
        // 閉じたタブは、文字を消してから高さを縮める。
        for c in &self.anims.closing {
            let elapsed = now.saturating_duration_since(c.started);
            let (text, _, _) = tab_close_progress(elapsed, reduced);
            let ghost = div()
                .h(px(row_height(None, Some(elapsed), reduced)))
                .flex_shrink_0()
                .overflow_hidden()
                .px(px(10.))
                .flex()
                .items_center()
                .whitespace_nowrap()
                .text_size(px(13.))
                .text_color(Rgba { a: theme.fg_dim.a * text, ..theme.fg_dim })
                .child(c.title.clone())
                .into_any_element();
            let at = c.index.min(rows.len());
            rows.insert(at, ghost);
        }
        let add = self
            .hoverable(div().id("vtabs-add"), HoverKey::AddTab, cx)
            .h(px(VTAB_H))
            .px(px(10.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .flex_shrink_0()
            .rounded(crate::theme::size::RADIUS_SM)
            .bg(self.hover_bg(HoverKey::AddTab, theme.bg_hover, now))
            .text_size(px(13.))
            .text_color(theme.fg_dim)
            .child(icon(IconName::Plus, theme.fg_dim))
            .child("新しいターミナル")
            .on_click(cx.listener(|this, _, _, cx| {
                this.new_default_tab();
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| this.open_shell_menu(ev.position, window, cx)),
            );
        let list = div()
            .id("vtabs")
            .w(px(VTABS_W))
            .h_full()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .overflow_y_scroll()
            .children(rows)
            .child(add);
        Some(
            div()
                .w(px(width))
                .h_full()
                .flex_shrink_0()
                .overflow_hidden()
                .border_r_1()
                .border_color(theme.border_subtle)
                .child(list)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{VTAB_H, row_height};
    use std::time::Duration;

    #[test]
    fn row_height_follows_tab_motion() {
        assert_eq!(row_height(None, None, false), VTAB_H);
        assert_eq!(row_height(Some(Duration::ZERO), None, false), 0.0);
        assert_eq!(row_height(Some(Duration::from_secs(1)), None, false), VTAB_H);
        // 動きを減らす設定では高さを動かさない。
        assert_eq!(row_height(Some(Duration::ZERO), None, true), VTAB_H);
        assert_eq!(row_height(None, Some(Duration::from_secs(1)), false), 0.0);
        assert_eq!(row_height(None, Some(Duration::ZERO), true), VTAB_H);
    }
}
