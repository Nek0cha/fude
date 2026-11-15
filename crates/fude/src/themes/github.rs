//! GitHub Dark と GitHub Dark Dimmed（primer/github-vscode-theme、MIT。licenses/github-vscode-theme-MIT.txt）。

use fude_term::Palette;

use super::{SyntaxPalette, ThemeColors};

pub const DARK: ThemeColors = ThemeColors {
    bg_window: 0x0d1117,
    bg_panel: 0x161b22,
    fg_default: 0xe6edf3,
    fg_dim: 0x7d8590,
    accent: 0x58a6ff,
    warn: 0xd29922,
    danger: 0xf85149,
    project_colors: [0x58a6ff, 0x39c5cf, 0xffa657, 0xbc8cff, 0x3fb950, 0xff7b72],
    diff_add: 0x3fb950,
    diff_del: 0xf85149,
    term: Palette {
        ansi: [
            0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4, //
            0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff,
        ],
        foreground: 0xe6edf3,
        background: 0x0d1117,
        cursor: 0x58a6ff,
    },
    syntax: Some(SyntaxPalette {
        keyword: 0xff7b72,
        string: 0xa5d6ff,
        comment: 0x8b949e,
        function: 0xd2a8ff,
        type_: 0xffa657,
        constant: 0x79c0ff,
        number: 0x79c0ff,
        variable: 0xffa657,
        property: 0x79c0ff,
        operator: 0xff7b72,
        punctuation: 0xe6edf3,
        tag: 0x7ee787,
        attribute: 0x79c0ff,
        title: 0x79c0ff,
        link: 0xa5d6ff,
    }),
};

pub const DARK_DIMMED: ThemeColors = ThemeColors {
    bg_window: 0x22272e,
    bg_panel: 0x2d333b,
    fg_default: 0xadbac7,
    fg_dim: 0x768390,
    accent: 0x539bf5,
    warn: 0xc69026,
    danger: 0xe5534b,
    project_colors: [0x539bf5, 0x39c5cf, 0xf69d50, 0xb083f0, 0x57ab5a, 0xf47067],
    diff_add: 0x57ab5a,
    diff_del: 0xe5534b,
    term: Palette {
        ansi: [
            0x545d68, 0xf47067, 0x57ab5a, 0xc69026, 0x539bf5, 0xb083f0, 0x39c5cf, 0x909dab, //
            0x636e7b, 0xff938a, 0x6bc46d, 0xdaaa3f, 0x6cb6ff, 0xdcbdfb, 0x56d4dd, 0xcdd9e5,
        ],
        foreground: 0xadbac7,
        background: 0x22272e,
        cursor: 0x539bf5,
    },
    syntax: Some(SyntaxPalette {
        keyword: 0xf47067,
        string: 0x96d0ff,
        comment: 0x768390,
        function: 0xdcbdfb,
        type_: 0xf69d50,
        constant: 0x6cb6ff,
        number: 0x6cb6ff,
        variable: 0xf69d50,
        property: 0x6cb6ff,
        operator: 0xf47067,
        punctuation: 0xadbac7,
        tag: 0x8ddb8c,
        attribute: 0x6cb6ff,
        title: 0x6cb6ff,
        link: 0x96d0ff,
    }),
};
