//! 端末の色を 0xRRGGBB に解決する。

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

/// 端末の色の表。色は 0xRRGGBB。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// ANSI の 16 色（0〜7 が通常、8〜15 が明るい色）。
    pub ansi: [u32; 16],
    pub foreground: u32,
    /// 反転とカーソル上の文字に使う、不透明な背景色。既定の背景は塗らない。
    pub background: u32,
    pub cursor: u32,
}

/// 6×6×6 の立方体の 1 辺の段階。
const CUBE_LEVELS: [u32; 6] = [0, 95, 135, 175, 215, 255];

fn rgb(c: Rgb) -> u32 {
    (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)
}

impl Palette {
    /// 番号（0〜255、または NamedColor の値）の色。端末側の上書きがあれば優先する。
    fn indexed(&self, index: usize, overrides: &Colors) -> u32 {
        if let Some(c) = overrides[index] {
            return rgb(c);
        }
        match index {
            0..=15 => self.ansi[index],
            16..=231 => {
                let i = index - 16;
                let (r, g, b) = (CUBE_LEVELS[i / 36], CUBE_LEVELS[(i / 6) % 6], CUBE_LEVELS[i % 6]);
                (r << 16) | (g << 8) | b
            }
            232..=255 => {
                let v = 8 + 10 * (index as u32 - 232);
                (v << 16) | (v << 8) | v
            }
            _ => self.foreground,
        }
    }

    fn named(&self, name: NamedColor, overrides: &Colors) -> u32 {
        match name {
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground => {
                overrides[NamedColor::Foreground as usize].map_or(self.foreground, rgb)
            }
            NamedColor::Background => overrides[NamedColor::Background as usize].map_or(self.background, rgb),
            NamedColor::Cursor => overrides[NamedColor::Cursor as usize].map_or(self.cursor, rgb),
            // Dim の 8 色は通常の 8 色で代用する（薄くするのは描画側の dim）。
            n if (n as usize) >= NamedColor::DimBlack as usize => {
                self.indexed(n as usize - NamedColor::DimBlack as usize, overrides)
            }
            n => self.indexed(n as usize, overrides),
        }
    }

    /// 前景色を解決する。
    pub fn resolve_fg(&self, color: Color, overrides: &Colors) -> u32 {
        match color {
            Color::Named(n) => self.named(n, overrides),
            Color::Spec(c) => rgb(c),
            Color::Indexed(i) => self.indexed(usize::from(i), overrides),
        }
    }

    /// 背景色を解決する。既定の背景（上書きも無いとき）は None ＝ 塗らない。
    pub fn resolve_bg(&self, color: Color, overrides: &Colors) -> Option<u32> {
        match color {
            Color::Named(NamedColor::Background) => overrides[NamedColor::Background as usize].map(rgb),
            other => Some(self.resolve_fg(other, overrides)),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alacritty_terminal::term::color::Colors;
    use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

    pub(crate) fn test_palette() -> Palette {
        Palette {
            ansi: [0x000000, 0x110000, 0x001100, 0x111100, 0x000011, 0x110011, 0x001111, 0x111111,
                   0x222222, 0x220000, 0x002200, 0x222200, 0x000022, 0x220022, 0x002222, 0x333333],
            foreground: 0xd8d8e2,
            background: 0x0e0e14,
            cursor: 0xb9a8ff,
        }
    }

    #[test]
    fn named_and_indexed_colors() {
        let p = test_palette();
        let c = Colors::default();
        assert_eq!(p.resolve_fg(Color::Named(NamedColor::Red), &c), p.ansi[1]);
        assert_eq!(p.resolve_fg(Color::Indexed(196), &c), 0xff0000);
        assert_eq!(p.resolve_fg(Color::Indexed(232), &c), 0x080808);
        assert_eq!(p.resolve_fg(Color::Spec(Rgb { r: 1, g: 2, b: 3 }), &c), 0x010203);
    }

    #[test]
    fn default_background_is_none() {
        let p = test_palette();
        assert_eq!(p.resolve_bg(Color::Named(NamedColor::Background), &Colors::default()), None);
    }
}
