//! Dracula（dracula/visual-studio-code、MIT。licenses/dracula-MIT.txt）。

use fude_term::Palette;

use super::{SyntaxPalette, ThemeColors};

pub const DRACULA: ThemeColors = ThemeColors {
    bg_window: 0x282a36,
    bg_panel: 0x21222c,
    fg_default: 0xf8f8f2,
    fg_dim: 0x6272a4,
    accent: 0xbd93f9,
    warn: 0xf1fa8c,
    danger: 0xff5555,
    project_colors: [0xbd93f9, 0x8be9fd, 0xffb86c, 0xff79c6, 0x50fa7b, 0xff5555],
    diff_add: 0x50fa7b,
    diff_del: 0xff5555,
    term: Palette {
        ansi: [
            0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2, //
            0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
        ],
        foreground: 0xf8f8f2,
        background: 0x282a36,
        cursor: 0xf8f8f2,
    },
    syntax: Some(SyntaxPalette {
        keyword: 0xff79c6,
        string: 0xf1fa8c,
        comment: 0x6272a4,
        function: 0x50fa7b,
        type_: 0x8be9fd,
        constant: 0xbd93f9,
        number: 0xbd93f9,
        variable: 0xf8f8f2,
        property: 0x8be9fd,
        operator: 0xff79c6,
        punctuation: 0xf8f8f2,
        tag: 0xff79c6,
        attribute: 0x50fa7b,
        title: 0xbd93f9,
        link: 0x8be9fd,
    }),
};
