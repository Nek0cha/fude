//! メイン領域。選択中のタブのペインを左右に並べる。計画3でターミナルの描画を入れる。

use fude_core::workspace::Tab;
use gpui_kit::*;

use crate::theme::{Theme, mono_font};

/// 選択中のタブのペインを横に均等に並べ、間に 1px の線を引く。タブが無ければ何も描かない。
pub fn render_main_area(tab: Option<&Tab>, cx: &App) -> Div {
    let theme = *cx.global::<Theme>();
    let Some(tab) = tab else { return div().size_full() };
    let mut row = div().size_full().flex().flex_row();
    for (i, pane) in tab.panes.iter().enumerate() {
        if i > 0 {
            row = row.child(div().w(px(1.)).h_full().bg(theme.border_subtle));
        }
        // ターミナルに置き換えるまでの仮の表示。
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .p(px(12.))
                .font(mono_font())
                .text_size(px(12.))
                .text_color(theme.fg_dim)
                .child(pane.kind.label()),
        );
    }
    row
}
