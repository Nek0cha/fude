//! メイン領域。「ターミナル｜ツリー｜エディタ列｜ピン列」を左から並べる（設計書 §4.1.3）。
//! 狭いときは、ツリーとエディタ列をターミナルの上に重ねて出す（columns の Place::Overlay）。
//! エディタ列を重ねるときは右端に寄せ、ピン列の上に出す。

use std::collections::HashMap;

use fude_core::workspace::{PaneId, Tab};
use gpui_kit::*;

use crate::columns::Place;
use crate::terminal_view::TerminalView;
use crate::theme::{Theme, popup_shadow};

/// ツリーかエディタ列の、描く中身と置き方。w は表示する幅（出入りの動きの途中も含む）。
pub(crate) struct ColumnEl {
    pub el: AnyElement,
    pub place: Place,
    pub w: Pixels,
    pub opacity: f32,
}

/// 選択中のタブのペインを横に均等に並べ、間に 1px の線を引く。その右にツリーとエディタ列を置く。
/// split_share は左のペインの幅の割合（分割の動きの途中も含む）。
pub(crate) fn render_main_area(
    tab: Option<&Tab>,
    terminals: &HashMap<PaneId, Entity<TerminalView>>,
    split_share: f32,
    tree: Option<ColumnEl>,
    editor: Option<ColumnEl>,
    pin: Option<ColumnEl>,
    cx: &App,
) -> Div {
    let theme = *cx.global::<Theme>();
    let line = || div().w(px(1.)).h_full().flex_shrink_0().bg(theme.border_subtle);
    let mut row = div().size_full().relative().flex().flex_row();
    // ターミナルが 0 枚なら、左端に線を引かない。
    let mut first = true;
    match tab {
        Some(tab) => {
            let mut terms = div().flex_1().min_w_0().h_full().flex().flex_row();
            for (i, pane) in tab.panes.iter().enumerate() {
                if i > 0 {
                    terms = terms.child(line());
                }
                // 左のペインは割合で幅を決め、右のペインは残りを使う。分割の解除の途中は、右に空きが残る。
                let mut cell = if i == 0 && (tab.panes.len() > 1 || split_share < 1.0) {
                    div().w(relative(split_share)).flex_shrink_0().h_full().overflow_hidden()
                } else {
                    div().flex_1().min_w_0().h_full().overflow_hidden()
                };
                if let Some(view) = terminals.get(&pane.id) {
                    cell = cell.child(view.clone());
                }
                terms = terms.child(cell);
            }
            row = row.child(terms);
            first = false;
        }
        // ターミナルが 0 枚なら、列の出入りの途中にできる隙間を右端に寄せる。
        None => row = row.child(div().flex_1()),
    }
    // 重ねた列は (順位, 要素) で集め、最後に順位順に描く。順位は、ピン列 0、ツリー 1、エディタ列 2（一番上）。
    let mut overlays: Vec<(u8, Div)> = Vec::new();
    // 重ねるツリーは、エディタ列の左隣に置く。エディタ列を並べているときは、その右にピン列（並べていれば）がある。
    // ピン列を重ねているときも、ツリーはピン列の左に置く（ピン列の幅は w で取る）。
    let pin_span = pin.as_ref().map_or(px(0.), |p| p.w);
    let editor_right = match editor.as_ref().map(|e| e.place) {
        Some(Place::Inline) => editor.as_ref().map_or(px(0.), |e| e.w) + pin_span,
        Some(Place::Overlay) => editor.as_ref().map_or(px(0.), |e| e.w),
        _ => pin_span,
    };
    let overlay = |c: ColumnEl, right: Pixels| {
        div()
            .absolute()
            .top(px(0.))
            .bottom(px(0.))
            .right(right)
            .w(c.w)
            .overflow_hidden()
            .opacity(c.opacity)
            .bg(theme.bg_panel)
            .border_l_1()
            .border_color(theme.border_subtle)
            .shadow(popup_shadow())
            .child(c.el)
    };
    // 並べる列は左から、ツリー、エディタ列、ピン列の順。rank は重ねるときの上下の順（大きいほど上）。
    for (c, rank) in [(tree, 1u8), (editor, 2), (pin, 0)] {
        let Some(c) = c else { continue };
        let place = c.place;
        match place {
            Place::Hidden => {}
            Place::Inline => {
                if !first {
                    row = row.child(line());
                }
                row = row.child(div().w(c.w).flex_shrink_0().h_full().overflow_hidden().opacity(c.opacity).child(c.el));
            }
            Place::Overlay => {
                let right = if rank == 1 { editor_right } else { px(0.) };
                overlays.push((rank, overlay(c, right)));
            }
        }
        if place == Place::Inline {
            first = false;
        }
    }
    // ピン列の上にツリー、その上にエディタ列を出す。
    overlays.sort_by_key(|(rank, _)| *rank);
    row.children(overlays.into_iter().map(|(_, d)| d))
}
