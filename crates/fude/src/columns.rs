//! メイン領域の列（ターミナル｜ツリー｜エディタ列｜ピン列）の幅の決め方（設計書 §4.1.3）。ターミナルを潰さないことを優先する。

pub(crate) const TREE_W: f32 = 240.0;
pub(crate) const EDITOR_MIN: f32 = 420.0;
pub(crate) const EDITOR_MAX: f32 = 900.0;
pub(crate) const EDITOR_SHARE: f32 = 0.45;
pub(crate) const TERMINAL_MIN: f32 = 480.0;
pub(crate) const PIN_W: f32 = 360.0;
/// ウィンドウの幅がこれ未満なら、エディタ列をピン列の上に重ねる。
pub(crate) const PIN_OVERLAY_BELOW: f32 = 1100.0;

/// 列の置き方。Overlay はターミナルの上に重ねて出す（ターミナルの幅は変えない）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    Hidden,
    Inline,
    Overlay,
}

/// 各列の置き方と幅（論理ピクセル）。Hidden の列の幅は 0。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Columns {
    pub terminal_w: f32,
    pub tree: Place,
    pub tree_w: f32,
    pub editor: Place,
    pub editor_w: f32,
    pub pin: Place,
    pub pin_w: f32,
}

/// 並べたときのエディタ列の幅。ターミナルに TERMINAL_MIN を残すよう縮めるが、EDITOR_MIN は割らない。
fn inline_editor_w(rest: f32) -> f32 {
    (rest * EDITOR_SHARE).clamp(EDITOR_MIN, EDITOR_MAX).min((rest - TERMINAL_MIN).max(EDITOR_MIN))
}

/// main_w はメイン領域の幅、window_w はウィンドウの幅。ピン列は開いていれば右端に並べ、残りで次の規則を使う。
/// ただし、ウィンドウが PIN_OVERLAY_BELOW より狭く、ターミナルとエディタ列があるときは、エディタ列をピン列の上に重ねる。
pub(crate) fn columns(main_w: f32, window_w: f32, has_terminals: bool, tree_open: bool, editor_open: bool, pin_open: bool) -> Columns {
    if !pin_open {
        return base_columns(main_w, has_terminals, tree_open, editor_open);
    }
    let rest = (main_w - PIN_W).max(0.0);
    if !has_terminals {
        // ターミナルが 0 枚: ツリーを並べるとエディタ列が EDITOR_MIN を割るなら、ツリーを重ねる。
        // ピン列の残りでも足りなければ、エディタ列をピン列の上に重ねる。
        let tree_inline = tree_open && rest - TREE_W >= EDITOR_MIN;
        let mut c = base_columns(rest, false, tree_inline, editor_open);
        if tree_open && !tree_inline {
            c.tree = Place::Overlay;
            c.tree_w = TREE_W;
            c.editor_w = rest;
        }
        if editor_open && rest < EDITOR_MIN {
            c.editor = Place::Overlay;
            c.editor_w = overlay_editor_w(main_w);
        }
        c.pin = Place::Inline;
        c.pin_w = PIN_W;
        return c;
    }
    // ピン列を並べるとターミナルが TERMINAL_MIN を割るなら、ピン列も右端に重ねる（ターミナルを押さない）。
    let pin_inline = rest >= TERMINAL_MIN;
    let over = editor_open && (window_w < PIN_OVERLAY_BELOW || !pin_inline);
    let mut c = base_columns(if pin_inline { rest } else { main_w }, true, tree_open, editor_open && !over);
    if over {
        c.editor = Place::Overlay;
        c.editor_w = overlay_editor_w(main_w);
    }
    c.pin = if pin_inline { Place::Inline } else { Place::Overlay };
    c.pin_w = PIN_W;
    c
}

/// 重ねて出すときのエディタ列の幅。
fn overlay_editor_w(main_w: f32) -> f32 {
    (main_w * EDITOR_SHARE).clamp(EDITOR_MIN, EDITOR_MAX).min(main_w)
}

/// ピン列を除いた規則。ターミナルが TERMINAL_MIN を割るときは、エディタ列を縮め、
/// 次にツリーを重ね、最後にエディタ列も重ねる。
fn base_columns(main_w: f32, has_terminals: bool, tree_open: bool, editor_open: bool) -> Columns {
    let hidden = (Place::Hidden, 0.0);
    let tree_w = if tree_open { TREE_W } else { 0.0 };
    let tree_place = if tree_open { Place::Inline } else { Place::Hidden };
    let editor_place = if editor_open { Place::Inline } else { Place::Hidden };
    if !has_terminals {
        let editor_w = if editor_open { (main_w - tree_w).max(0.0) } else { 0.0 };
        return Columns { terminal_w: 0.0, tree: tree_place, tree_w, editor: editor_place, editor_w, pin: hidden.0, pin_w: hidden.1 };
    }
    // ツリーを並べたまま、次にツリーを重ねて、の順に試す。
    for tree_inline in [true, false] {
        if !tree_inline && !tree_open {
            break;
        }
        let rest = main_w - if tree_inline { tree_w } else { 0.0 };
        let editor_w = if editor_open { inline_editor_w(rest) } else { 0.0 };
        let terminal_w = rest - editor_w;
        if terminal_w >= TERMINAL_MIN {
            let tree = if tree_inline { tree_place } else { Place::Overlay };
            return Columns { terminal_w, tree, tree_w, editor: editor_place, editor_w, pin: hidden.0, pin_w: hidden.1 };
        }
    }
    let tree = if tree_open { Place::Overlay } else { Place::Hidden };
    let (editor, editor_w) = if editor_open {
        (Place::Overlay, overlay_editor_w(main_w))
    } else {
        (Place::Hidden, 0.0)
    };
    Columns { terminal_w: main_w, tree, tree_w, editor, editor_w, pin: hidden.0, pin_w: hidden.1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_places_all_inline() {
        let c = columns(1600.0, 1600.0, true, true, true, false);
        assert_eq!((c.tree, c.editor), (Place::Inline, Place::Inline));
        assert_eq!(c.tree_w, 240.0);
        assert!((c.editor_w - 612.0).abs() < 0.01);
        assert!((c.terminal_w - 748.0).abs() < 0.01);
    }

    #[test]
    fn editor_shrinks_to_keep_terminal() {
        // 1148 = 1200 − レール 52。残り 908、45% は 408.6 → 420、ターミナル 488。
        let c = columns(1148.0, 1600.0, true, true, true, false);
        assert_eq!((c.tree, c.editor), (Place::Inline, Place::Inline));
        assert_eq!(c.editor_w, 420.0);
        assert!(c.terminal_w >= 480.0);
    }

    #[test]
    fn narrow_overlays_tree_then_editor() {
        let c = columns(1000.0, 1600.0, true, true, true, false);
        assert_eq!((c.tree, c.editor), (Place::Overlay, Place::Inline));
        assert_eq!(c.editor_w, 450.0);
        assert_eq!(c.terminal_w, 550.0);
        let c = columns(800.0, 1600.0, true, true, true, false);
        assert_eq!((c.tree, c.editor), (Place::Overlay, Place::Overlay));
        assert_eq!(c.terminal_w, 800.0);
        assert_eq!(c.editor_w, 420.0);
    }

    #[test]
    fn tree_alone_sits_inline_when_room() {
        let c = columns(1148.0, 1600.0, true, true, false, false);
        assert_eq!((c.tree, c.editor), (Place::Inline, Place::Hidden));
        assert_eq!(c.terminal_w, 908.0);
        let c = columns(600.0, 1600.0, true, true, false, false);
        assert_eq!(c.tree, Place::Overlay);
        assert_eq!(c.terminal_w, 600.0);
    }

    #[test]
    fn no_terminals_editor_fills() {
        let c = columns(1148.0, 1600.0, false, true, true, false);
        assert_eq!((c.tree, c.editor), (Place::Inline, Place::Inline));
        assert_eq!(c.editor_w, 908.0);
        assert_eq!(c.terminal_w, 0.0);
        assert_eq!(columns(1148.0, 1600.0, false, false, true, false).editor_w, 1148.0);
    }

    #[test]
    fn pin_takes_right_edge() {
        let c = columns(1500.0, 1600.0, true, false, true, true);
        assert_eq!((c.pin, c.pin_w), (Place::Inline, PIN_W));
        assert_eq!(c.editor, Place::Inline);
        assert!((c.terminal_w + c.editor_w + c.pin_w - 1500.0).abs() < 0.01);
    }

    #[test]
    fn narrow_window_overlays_editor_on_pin() {
        let c = columns(950.0, 1000.0, true, false, true, true);
        assert_eq!(c.pin, Place::Inline);
        assert_eq!(c.editor, Place::Overlay);
        assert_eq!(c.terminal_w, 950.0 - PIN_W);
        assert!(c.terminal_w >= TERMINAL_MIN);
    }

    #[test]
    fn pin_without_terminals() {
        let c = columns(1200.0, 1300.0, false, false, true, true);
        assert_eq!(c.editor_w, 1200.0 - PIN_W);
        let only_pin = columns(1200.0, 1300.0, false, false, false, true);
        assert_eq!((only_pin.editor, only_pin.pin), (Place::Hidden, Place::Inline));
    }

    /// ピン列を並べるとターミナルが TERMINAL_MIN を割るときは、ピン列も右端に重ねる。
    #[test]
    fn pin_overlays_when_terminal_would_shrink() {
        let c = columns(588.0, 640.0, true, false, false, true);
        assert_eq!(c.pin, Place::Overlay);
        assert_eq!(c.terminal_w, 588.0);
        let c = columns(588.0, 640.0, true, false, true, true);
        assert_eq!((c.editor, c.pin), (Place::Overlay, Place::Overlay));
        assert_eq!(c.terminal_w, 588.0);
        // ウィンドウ 1000px で縦のタブ（160px）を出したとき。
        let c = columns(788.0, 1000.0, true, false, true, true);
        assert!(c.terminal_w >= TERMINAL_MIN);
        assert_eq!(c.pin, Place::Overlay);
    }

    /// ターミナルが 0 枚でも、エディタ列が EDITOR_MIN を割らない（先にツリーを重ね、次にエディタ列をピン列に重ねる）。
    #[test]
    fn pin_without_terminals_keeps_editor_min() {
        let c = columns(848.0, 900.0, false, true, true, true);
        assert_eq!(c.tree, Place::Overlay);
        assert_eq!(c.editor, Place::Inline);
        assert_eq!(c.editor_w, 848.0 - PIN_W);
        let c = columns(588.0, 640.0, false, true, true, true);
        assert_eq!(c.editor, Place::Overlay);
        assert!(c.editor_w >= EDITOR_MIN);
    }
}
