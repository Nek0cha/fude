//! メイン領域の列（ターミナル｜ツリー｜エディタ列）の幅の決め方（設計書 §4.1.3）。ターミナルを潰さないことを優先する。

pub(crate) const TREE_W: f32 = 240.0;
pub(crate) const EDITOR_MIN: f32 = 420.0;
pub(crate) const EDITOR_MAX: f32 = 900.0;
pub(crate) const EDITOR_SHARE: f32 = 0.45;
pub(crate) const TERMINAL_MIN: f32 = 480.0;

/// 列の置き方。Overlay はターミナルの上に重ねて出す（ターミナルの幅は変えない）。
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    Hidden,
    Inline,
    Overlay,
}

/// 各列の置き方と幅（論理ピクセル）。Hidden の列の幅は 0。
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Columns {
    pub terminal_w: f32,
    pub tree: Place,
    pub tree_w: f32,
    pub editor: Place,
    pub editor_w: f32,
}

/// 並べたときのエディタ列の幅。ターミナルに TERMINAL_MIN を残すよう縮めるが、EDITOR_MIN は割らない。
fn inline_editor_w(rest: f32) -> f32 {
    (rest * EDITOR_SHARE).clamp(EDITOR_MIN, EDITOR_MAX).min((rest - TERMINAL_MIN).max(EDITOR_MIN))
}

/// main_w はメイン領域の幅。ターミナルが TERMINAL_MIN を割るときは、エディタ列を縮め、
/// 次にツリーを重ね、最後にエディタ列も重ねる。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn columns(main_w: f32, has_terminals: bool, tree_open: bool, editor_open: bool) -> Columns {
    let tree_w = if tree_open { TREE_W } else { 0.0 };
    let tree_place = if tree_open { Place::Inline } else { Place::Hidden };
    let editor_place = if editor_open { Place::Inline } else { Place::Hidden };
    if !has_terminals {
        let editor_w = if editor_open { (main_w - tree_w).max(0.0) } else { 0.0 };
        return Columns { terminal_w: 0.0, tree: tree_place, tree_w, editor: editor_place, editor_w };
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
            return Columns { terminal_w, tree, tree_w, editor: editor_place, editor_w };
        }
    }
    let tree = if tree_open { Place::Overlay } else { Place::Hidden };
    let (editor, editor_w) = if editor_open {
        (Place::Overlay, (main_w * EDITOR_SHARE).clamp(EDITOR_MIN, EDITOR_MAX).min(main_w))
    } else {
        (Place::Hidden, 0.0)
    };
    Columns { terminal_w: main_w, tree, tree_w, editor, editor_w }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_places_all_inline() {
        let c = columns(1600.0, true, true, true);
        assert_eq!((c.tree, c.editor), (Place::Inline, Place::Inline));
        assert_eq!(c.tree_w, 240.0);
        assert!((c.editor_w - 612.0).abs() < 0.01);
        assert!((c.terminal_w - 748.0).abs() < 0.01);
    }

    #[test]
    fn editor_shrinks_to_keep_terminal() {
        // 1148 = 1200 − レール 52。残り 908、45% は 408.6 → 420、ターミナル 488。
        let c = columns(1148.0, true, true, true);
        assert_eq!((c.tree, c.editor), (Place::Inline, Place::Inline));
        assert_eq!(c.editor_w, 420.0);
        assert!(c.terminal_w >= 480.0);
    }

    #[test]
    fn narrow_overlays_tree_then_editor() {
        let c = columns(1000.0, true, true, true);
        assert_eq!((c.tree, c.editor), (Place::Overlay, Place::Inline));
        assert_eq!(c.editor_w, 450.0);
        assert_eq!(c.terminal_w, 550.0);
        let c = columns(800.0, true, true, true);
        assert_eq!((c.tree, c.editor), (Place::Overlay, Place::Overlay));
        assert_eq!(c.terminal_w, 800.0);
        assert_eq!(c.editor_w, 420.0);
    }

    #[test]
    fn tree_alone_sits_inline_when_room() {
        let c = columns(1148.0, true, true, false);
        assert_eq!((c.tree, c.editor), (Place::Inline, Place::Hidden));
        assert_eq!(c.terminal_w, 908.0);
        let c = columns(600.0, true, true, false);
        assert_eq!(c.tree, Place::Overlay);
        assert_eq!(c.terminal_w, 600.0);
    }

    #[test]
    fn no_terminals_editor_fills() {
        let c = columns(1148.0, false, true, true);
        assert_eq!((c.tree, c.editor), (Place::Inline, Place::Inline));
        assert_eq!(c.editor_w, 908.0);
        assert_eq!(c.terminal_w, 0.0);
        assert_eq!(columns(1148.0, false, false, true).editor_w, 1148.0);
    }
}
