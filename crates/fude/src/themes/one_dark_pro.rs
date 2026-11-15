//! One Dark Pro（Binaryify/OneDark-Pro、MIT。licenses/one-dark-pro-MIT.txt）。

use fude_term::Palette;

use super::{SyntaxPalette, ThemeColors};

pub const ONE_DARK_PRO: ThemeColors = ThemeColors {
    bg_window: 0x282c34,
    bg_panel: 0x21252b,
    fg_default: 0xabb2bf,
    fg_dim: 0x7f848e,
    accent: 0x61afef,
    warn: 0xe5c07b,
    danger: 0xe06c75,
    project_colors: [0x61afef, 0x56b6c2, 0xd19a66, 0xc678dd, 0x98c379, 0xe06c75],
    diff_add: 0x98c379,
    diff_del: 0xe06c75,
    term: Palette {
        ansi: [
            0x3f4451, 0xe05561, 0x8cc265, 0xd18f52, 0x4aa5f0, 0xc162de, 0x42b3c2, 0xd7dae0, //
            0x4f5666, 0xff616e, 0xa5e075, 0xf0a45d, 0x4dc4ff, 0xde73ff, 0x4cd1e0, 0xe6e6e6,
        ],
        foreground: 0xabb2bf,
        background: 0x282c34,
        cursor: 0x528bff,
    },
    syntax: Some(SyntaxPalette {
        keyword: 0xc678dd,
        string: 0x98c379,
        comment: 0x7f848e,
        function: 0x61afef,
        type_: 0xe5c07b,
        constant: 0xd19a66,
        number: 0xd19a66,
        variable: 0xe06c75,
        property: 0xe06c75,
        operator: 0x56b6c2,
        punctuation: 0xabb2bf,
        tag: 0xe06c75,
        attribute: 0xd19a66,
        title: 0xe06c75,
        link: 0x61afef,
    }),
};
