//! メイン領域。選択中のタブのペインを左右に並べ、右パネルがあれば右に置く。タブが 0 枚なら、右パネルが全体を使う。

use std::collections::HashMap;

use fude_core::workspace::{PaneId, Tab};
use gpui_kit::*;

use crate::right_panel::panel_width;
use crate::terminal_view::TerminalView;
use crate::theme::Theme;

/// メイン領域の並び方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    Terminals,
    TerminalsAndPanel,
    /// ターミナルが 0 枚で、右パネルの中身がメイン領域を全部使う（設計書 §4.1.1）。
    PanelOnly,
    Empty,
}

pub(crate) fn layout_for(has_tabs: bool, panel_open: bool) -> Layout {
    match (has_tabs, panel_open) {
        (true, false) => Layout::Terminals,
        (true, true) => Layout::TerminalsAndPanel,
        (false, true) => Layout::PanelOnly,
        (false, false) => Layout::Empty,
    }
}

/// 選択中のタブのペインを横に均等に並べ、間に 1px の線を引く。panel があれば右に置く。main_w はメイン領域の幅。
pub fn render_main_area(
    tab: Option<&Tab>,
    terminals: &HashMap<PaneId, Entity<TerminalView>>,
    panel: Option<AnyView>,
    main_w: Pixels,
    cx: &App,
) -> Div {
    let theme = *cx.global::<Theme>();
    let line = || div().w(px(1.)).h_full().flex_shrink_0().bg(theme.border_subtle);
    let terminals_row = |tab: &Tab| {
        let mut row = div().flex_1().min_w_0().h_full().flex().flex_row();
        for (i, pane) in tab.panes.iter().enumerate() {
            if i > 0 {
                row = row.child(line());
            }
            let mut cell = div().flex_1().min_w_0().h_full().overflow_hidden();
            if let Some(view) = terminals.get(&pane.id) {
                cell = cell.child(view.clone());
            }
            row = row.child(cell);
        }
        row
    };
    match (layout_for(tab.is_some(), panel.is_some()), tab, panel) {
        (Layout::Terminals, Some(tab), _) => div().size_full().flex().child(terminals_row(tab)),
        (Layout::TerminalsAndPanel, Some(tab), Some(panel)) => div()
            .size_full()
            .flex()
            .flex_row()
            .child(terminals_row(tab))
            .child(line())
            .child(div().w(panel_width(main_w)).flex_shrink_0().h_full().child(panel)),
        (Layout::PanelOnly, _, Some(panel)) => div().size_full().child(panel),
        _ => div().size_full(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Layout, layout_for};

    #[test]
    fn layout_rules() {
        assert_eq!(layout_for(true, false), Layout::Terminals);
        assert_eq!(layout_for(true, true), Layout::TerminalsAndPanel);
        assert_eq!(layout_for(false, true), Layout::PanelOnly);
        assert_eq!(layout_for(false, false), Layout::Empty);
    }
}
