//! 右クリックで出すメニューの共通部分。位置、出入りの動き、フォーカス、枠の見た目を持つ。

use std::time::Instant;

use fude_core::motion::Fade;
use gpui_kit::*;

use crate::theme::motion::{POPUP_IN, POPUP_OUT, ease_out_quint};
use crate::theme::{Theme, popup_shadow, size, ui_font};

pub(crate) const ITEM_H: Pixels = px(28.);
const MENU_MIN_W: Pixels = px(180.);

/// 開いているメニューの状態。消える動きの途中も持ち続ける。
pub(crate) struct Popup {
    position: Point<Pixels>,
    fade: Fade,
    closing: bool,
    focus: FocusHandle,
}

impl Popup {
    /// position を左上にして出す。フォーカスをメニューに移す（Esc を受けるため）。
    pub fn open(position: Point<Pixels>, window: &mut Window, cx: &mut App) -> Popup {
        let mut fade = Fade::settled(0.0);
        fade.animate_to(1.0, POPUP_IN, ease_out_quint, Instant::now());
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Popup { position, fade, closing: false, focus }
    }

    /// 消える動きを始める。2 回目は何もしない。始めたら true。
    pub fn close(&mut self, now: Instant) -> bool {
        if self.closing {
            return false;
        }
        self.closing = true;
        self.fade.animate_to(0.0, POPUP_OUT, ease_out_quint, now);
        true
    }

    pub fn closing(&self) -> bool {
        self.closing
    }

    /// 消え終わった。
    pub fn is_gone(&self, now: Instant) -> bool {
        self.closing && !self.fade.is_animating(now)
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.fade.is_animating(now)
    }

    pub fn position(&self) -> Point<Pixels> {
        self.position
    }

    /// メニューの枠（背景、影、角丸、不透明度、フォーカス）。Esc と外側のクリックの処理、中身は呼び出し側が足す。
    pub fn frame(&self, id: &'static str, theme: &Theme, now: Instant) -> Stateful<Div> {
        div()
            .id(id)
            // 下の部品（ターミナルなど）にクリックを渡さない。
            .occlude()
            .track_focus(&self.focus)
            .opacity(self.fade.value(now))
            .min_w(MENU_MIN_W)
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(size::RADIUS_MD)
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border_subtle)
            .shadow(popup_shadow())
            .font(ui_font())
            .text_size(px(13.))
            .text_color(theme.fg_default)
    }
}

/// 枠を position に置き、ほかの要素より手前に描く。
pub(crate) fn place(position: Point<Pixels>, body: impl IntoElement) -> Deferred {
    deferred(anchored().position(position).child(body)).with_priority(1)
}
