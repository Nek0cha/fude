//! メイン領域。選択中のタブのペインを左右に並べ、それぞれにターミナルを置く。

use std::collections::HashMap;

use fude_core::workspace::{PaneId, Tab};
use gpui_kit::*;

use crate::terminal_view::TerminalView;
use crate::theme::Theme;

/// 選択中のタブのペインを横に均等に並べ、間に 1px の線を引く。タブが無ければ何も描かない。
pub fn render_main_area(tab: Option<&Tab>, terminals: &HashMap<PaneId, Entity<TerminalView>>, cx: &App) -> Div {
    let theme = *cx.global::<Theme>();
    let Some(tab) = tab else { return div().size_full() };
    let mut row = div().size_full().flex().flex_row();
    for (i, pane) in tab.panes.iter().enumerate() {
        if i > 0 {
            row = row.child(div().w(px(1.)).h_full().bg(theme.border_subtle));
        }
        let mut cell = div().flex_1().min_w_0().h_full();
        if let Some(view) = terminals.get(&pane.id) {
            cell = cell.child(view.clone());
        }
        row = row.child(cell);
    }
    row
}
