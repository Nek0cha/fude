//! fude の配色（計画9a より前の Theme::dark() と同じ値）と、その地を真っ黒にした OLED。

use fude_term::Palette;

use super::ThemeColors;

pub const FUDE: ThemeColors = ThemeColors {
    bg_window: 0x0e0e14,
    bg_panel: 0x1e1e2a,
    fg_default: 0xd8d8e2,
    fg_dim: 0x7b7d90,
    accent: 0xb9a8ff,
    warn: 0xffd28a,
    danger: 0xff8a8a,
    project_colors: [0x8fa7ff, 0x7fd1c7, 0xe6b07a, 0xd99ad6, 0x9fd38a, 0xe59a9a],
    diff_add: 0x5fe0b0,
    diff_del: 0xff8a8a,
    term: Palette {
        ansi: [
            0x1e1e2a, 0xff8a8a, 0x5fe0b0, 0xffd28a, 0x8fa7ff, 0xd99ad6, 0x7fd1c7, 0xd8d8e2, //
            0x7b7d90, 0xffa3a3, 0x84ecc4, 0xffe0a8, 0xaabcff, 0xe8b4e6, 0x9fe3da, 0xf2f2f7,
        ],
        foreground: 0xd8d8e2,
        background: 0x0e0e14,
        cursor: 0xb9a8ff,
    },
    syntax: None,
};

/// 地とターミナルの背景を真っ黒にし、パネルはほぼ黒にする。ほかは fude と同じ。
pub const OLED: ThemeColors = ThemeColors {
    bg_window: 0x000000,
    bg_panel: 0x0a0a0a,
    term: Palette { background: 0x000000, ..FUDE.term },
    ..FUDE
};
