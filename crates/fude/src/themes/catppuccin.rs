//! Catppuccin の Mocha、Macchiato、Frappé（catppuccin/vscode、MIT。licenses/catppuccin-MIT.txt）。
//! アクセントは既定の mauve。

use fude_term::Palette;

use super::{SyntaxPalette, ThemeColors};

/// 1 つの味（flavor）の色の名前。
struct Flavor {
    base: u32,
    mantle: u32,
    text: u32,
    subtext1: u32,
    subtext0: u32,
    overlay2: u32,
    overlay0: u32,
    surface2: u32,
    surface1: u32,
    rosewater: u32,
    mauve: u32,
    red: u32,
    peach: u32,
    yellow: u32,
    green: u32,
    teal: u32,
    sky: u32,
    blue: u32,
    lavender: u32,
    pink: u32,
}

const fn theme(f: Flavor) -> ThemeColors {
    ThemeColors {
        bg_window: f.base,
        bg_panel: f.mantle,
        fg_default: f.text,
        fg_dim: f.overlay0,
        accent: f.mauve,
        warn: f.yellow,
        danger: f.red,
        project_colors: [f.blue, f.teal, f.peach, f.pink, f.green, f.red],
        diff_add: f.green,
        diff_del: f.red,
        term: Palette {
            ansi: [
                f.surface1, f.red, f.green, f.yellow, f.blue, f.pink, f.teal, f.subtext1, //
                f.surface2, f.red, f.green, f.yellow, f.blue, f.pink, f.teal, f.subtext0,
            ],
            foreground: f.text,
            background: f.base,
            cursor: f.rosewater,
        },
        syntax: Some(SyntaxPalette {
            keyword: f.mauve,
            string: f.green,
            comment: f.overlay2,
            function: f.blue,
            type_: f.yellow,
            constant: f.peach,
            number: f.peach,
            variable: f.text,
            property: f.lavender,
            operator: f.sky,
            punctuation: f.overlay2,
            tag: f.blue,
            attribute: f.yellow,
            title: f.red,
            link: f.lavender,
        }),
    }
}

pub const MOCHA: ThemeColors = theme(Flavor {
    base: 0x1e1e2e,
    mantle: 0x181825,
    text: 0xcdd6f4,
    subtext1: 0xbac2de,
    subtext0: 0xa6adc8,
    overlay2: 0x9399b2,
    overlay0: 0x6c7086,
    surface2: 0x585b70,
    surface1: 0x45475a,
    rosewater: 0xf5e0dc,
    mauve: 0xcba6f7,
    red: 0xf38ba8,
    peach: 0xfab387,
    yellow: 0xf9e2af,
    green: 0xa6e3a1,
    teal: 0x94e2d5,
    sky: 0x89dceb,
    blue: 0x89b4fa,
    lavender: 0xb4befe,
    pink: 0xf5c2e7,
});

pub const MACCHIATO: ThemeColors = theme(Flavor {
    base: 0x24273a,
    mantle: 0x1e2030,
    text: 0xcad3f5,
    subtext1: 0xb8c0e0,
    subtext0: 0xa5adcb,
    overlay2: 0x939ab7,
    overlay0: 0x6e738d,
    surface2: 0x5b6078,
    surface1: 0x494d64,
    rosewater: 0xf4dbd6,
    mauve: 0xc6a0f6,
    red: 0xed8796,
    peach: 0xf5a97f,
    yellow: 0xeed49f,
    green: 0xa6da95,
    teal: 0x8bd5ca,
    sky: 0x91d7e3,
    blue: 0x8aadf4,
    lavender: 0xb7bdf8,
    pink: 0xf5bde6,
});

pub const FRAPPE: ThemeColors = theme(Flavor {
    base: 0x303446,
    mantle: 0x292c3c,
    text: 0xc6d0f5,
    subtext1: 0xb5bfe2,
    subtext0: 0xa5adce,
    overlay2: 0x949cbb,
    overlay0: 0x737994,
    surface2: 0x626880,
    surface1: 0x51576d,
    rosewater: 0xf2d5cf,
    mauve: 0xca9ee6,
    red: 0xe78284,
    peach: 0xef9f76,
    yellow: 0xe5c890,
    green: 0xa6d189,
    teal: 0x81c8be,
    sky: 0x99d1db,
    blue: 0x8caaee,
    lavender: 0xbabbf1,
    pink: 0xf4b8e4,
});
