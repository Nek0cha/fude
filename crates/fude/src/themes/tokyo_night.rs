//! Tokyo Night と Tokyo Night Storm（enkia/tokyo-night-vscode-theme、MIT。licenses/tokyo-night-MIT.txt）。

use fude_term::Palette;

use super::{SyntaxPalette, ThemeColors};

const SYNTAX: SyntaxPalette = SyntaxPalette {
    keyword: 0xbb9af7,
    string: 0x9ece6a,
    comment: 0x565f89,
    function: 0x7aa2f7,
    type_: 0x2ac3de,
    constant: 0xff9e64,
    number: 0xff9e64,
    variable: 0xc0caf5,
    property: 0x73daca,
    operator: 0x89ddff,
    punctuation: 0x9abdf5,
    tag: 0xf7768e,
    attribute: 0xbb9af7,
    title: 0x89ddff,
    link: 0x73daca,
};

pub const NIGHT: ThemeColors = ThemeColors {
    bg_window: 0x1a1b26,
    bg_panel: 0x16161e,
    fg_default: 0xa9b1d6,
    fg_dim: 0x565f89,
    accent: 0x7aa2f7,
    warn: 0xe0af68,
    danger: 0xf7768e,
    project_colors: [0x7aa2f7, 0x7dcfff, 0xff9e64, 0xbb9af7, 0x9ece6a, 0xf7768e],
    diff_add: 0x9ece6a,
    diff_del: 0xf7768e,
    term: Palette {
        ansi: [
            0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, //
            0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
        ],
        foreground: 0xa9b1d6,
        background: 0x1a1b26,
        cursor: 0xc0caf5,
    },
    syntax: Some(SYNTAX),
};

pub const STORM: ThemeColors = ThemeColors {
    bg_window: 0x24283b,
    bg_panel: 0x1f2335,
    term: Palette {
        ansi: [
            0x1d202f, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, //
            0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
        ],
        background: 0x24283b,
        ..NIGHT.term
    },
    ..NIGHT
};
