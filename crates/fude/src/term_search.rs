//! ターミナル内の検索の欄（設計書 §5.6）。ターミナルの右上に重ねる。文字どおり、スマートケースで探す。

use std::time::Instant;

use fude_core::motion::Fade;
use fude_term::state::TERM_SEARCH_MAX;
use gpui_kit::assets::IconName;
use gpui_kit::base::input::InputState;
use gpui_kit::component::input::Input;
use gpui_kit::*;

use crate::theme::{Theme, mono_font, popup_shadow, size};
use crate::title_row::icon;

gpui_kit::actions!(term_search, [TermSearchClose]);

/// 検索の欄の入力の文脈。gpui-kit の入力欄の Esc より優先させる。
const TERM_SEARCH_INPUT: &str = "TermSearch > Input";

/// 欄の幅と、ターミナルの右上からの距離。
pub(crate) const BAR_W: Pixels = px(280.);
pub(crate) const BAR_RIGHT: Pixels = px(12.);
pub(crate) const BAR_TOP: Pixels = px(8.);

pub fn bind_term_search_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", TermSearchClose, Some(TERM_SEARCH_INPUT))]);
}

/// 「3/12」の表示。空の入力では何も出さない。上限で打ち切ったら「+」を付ける。
pub(crate) fn count_label(counts: (usize, usize), query: &str) -> String {
    match counts {
        _ if query.is_empty() => String::new(),
        (_, 0) => "0 件".into(),
        (i, n) if n >= TERM_SEARCH_MAX => format!("{i}/{n}+"),
        (i, n) => format!("{i}/{n}"),
    }
}

/// 出ている検索の欄。消える動きの間も持つ。
pub(crate) struct TermSearchBar {
    pub input: Entity<InputState>,
    /// （選んだ一致の番号 1 始まり, 一致の数）。
    pub counts: (usize, usize),
    pub fade: Fade,
    pub closing: bool,
    pub _sub: Subscription,
}

/// 欄の小さなボタン（↑ ↓ ×）。
pub(crate) fn bar_button(id: &'static str, name: IconName, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .hover(|s| s.bg(theme.bg_hover))
        .child(icon(name, theme.fg_dim))
}

impl TermSearchBar {
    /// 欄の枠と中身（ボタンの動きは呼び出し側が on_click で足す）。
    pub(crate) fn frame(&self, query: &str, theme: &Theme, now: Instant) -> Stateful<Div> {
        let label = count_label(self.counts, query);
        let label_color = if self.counts.1 == 0 { theme.fg_dim } else { theme.fg_default };
        div()
            .id("term-search")
            .key_context("TermSearch")
            .occlude()
            .absolute()
            .top(BAR_TOP)
            .right(BAR_RIGHT)
            .w(BAR_W)
            .h(px(34.))
            .px(px(6.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .opacity(self.fade.value(now))
            .rounded(size::RADIUS_MD)
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border_subtle)
            .shadow(popup_shadow())
            .text_size(px(12.))
            .text_color(theme.fg_default)
            .child(div().flex_1().min_w_0().font(mono_font()).child(Input::new(&self.input).appearance(false)))
            .child(div().flex_shrink_0().px(px(4.)).text_color(label_color).child(label))
    }
}

#[cfg(test)]
mod tests {
    use super::count_label;
    use fude_term::state::TERM_SEARCH_MAX;

    #[test]
    fn count_label_shows_position_and_total() {
        assert_eq!(count_label((0, 0), ""), "");
        assert_eq!(count_label((0, 0), "x"), "0 件");
        assert_eq!(count_label((3, 12), "x"), "3/12");
        assert_eq!(count_label((1, TERM_SEARCH_MAX), "x"), "1/1000+");
    }
}
