//! 色、寸法、動き、フォントのトークン（設計書 §6.2、§6.3）。
//! 描画コードには色や寸法の値を直接書かず、ここを通す。

use fude_term::Palette;
use gpui_kit::{BoxShadow, Font, FontFallbacks, Global, Rgba, font, hsla, point, px, rgb, rgba};

pub const MONO_FAMILY: &str = "HackGen Console NF";

#[derive(Clone, Copy)]
#[allow(dead_code)] // bg_panel、warn、danger は後続の計画（パネル、返事待ち）で使う
pub struct Theme {
    pub bg_window: Rgba,
    pub bg_panel: Rgba,
    pub bg_hover: Rgba,
    pub border_subtle: Rgba,
    pub fg_default: Rgba,
    pub fg_dim: Rgba,
    pub accent: Rgba,
    pub warn: Rgba,
    pub danger: Rgba,
    /// Windows の閉じるボタンのホバー。danger の色を薄く重ねる（OS 標準の濃い赤は強すぎるため）。
    pub window_close_hover: Rgba,
    /// プロジェクトのアイコンの色。明度と彩度を揃えた 6 色。
    pub project_colors: [Rgba; 6],
    /// ターミナルの色。既存のトークンと同じ明度の系統にそろえる。
    pub term_palette: Palette,
    /// ターミナルの選択範囲。accent の 30%。
    pub selection: Rgba,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            bg_window: rgba(0x0e0e149e),
            bg_panel: rgba(0x1e1e2aeb),
            bg_hover: rgba(0xffffff12),
            border_subtle: rgba(0xffffff1f),
            fg_default: rgb(0xd8d8e2),
            fg_dim: rgb(0x7b7d90),
            accent: rgb(0xb9a8ff),
            warn: rgb(0xffd28a),
            danger: rgb(0xff8a8a),
            window_close_hover: rgba(0xff8a8a38),
            project_colors: [
                rgba(0x8fa7ff47),
                rgba(0x7fd1c747),
                rgba(0xe6b07a47),
                rgba(0xd99ad647),
                rgba(0x9fd38a47),
                rgba(0xe59a9a47),
            ],
            term_palette: Palette {
                ansi: [
                    0x1e1e2a, 0xff8a8a, 0x5fe0b0, 0xffd28a, 0x8fa7ff, 0xd99ad6, 0x7fd1c7, 0xd8d8e2, //
                    0x7b7d90, 0xffa3a3, 0x84ecc4, 0xffe0a8, 0xaabcff, 0xe8b4e6, 0x9fe3da, 0xf2f2f7,
                ],
                foreground: 0xd8d8e2,
                background: 0x0e0e14,
                cursor: 0xb9a8ff,
            },
            selection: rgba(0xb9a8ff4d),
        }
    }
}

impl Global for Theme {}

/// 寸法のトークン。
pub mod size {
    use gpui_kit::{Pixels, px};

    pub const TITLE_ROW_H: Pixels = px(38.);
    pub const RAIL_W: Pixels = px(52.);
    /// タイトル行の左端の空き。macOS は信号機ボタンの場所を取る。
    pub const TITLE_LEAD_W: Pixels = if cfg!(target_os = "macos") { px(92.) } else { RAIL_W };
    pub const TAB_H: Pixels = px(28.);
    pub const TAB_MAX_W: Pixels = px(200.);
    pub const CAPTION_BTN_W: Pixels = px(46.);
    pub const PROJECT_ICON: Pixels = px(32.);
    pub const RADIUS_SM: Pixels = px(6.);
    pub const RADIUS_MD: Pixels = px(8.);
    pub const TERM_FONT_SIZE: Pixels = px(14.);
    pub const TERM_LINE_HEIGHT: Pixels = px(18.);
    pub const TERM_PAD_X: Pixels = px(12.);
    pub const TERM_PAD_Y: Pixels = px(8.);
}

/// 動きのトークン。
pub mod motion {
    use std::time::Duration;

    pub const HOVER_IN: Duration = Duration::from_millis(80);
    pub const HOVER_OUT: Duration = Duration::from_millis(120);

    pub fn ease_out(t: f32) -> f32 {
        1.0 - (1.0 - t) * (1.0 - t)
    }

    /// ポップアップ（メニュー）の出るときと消えるとき。
    pub const POPUP_IN: Duration = Duration::from_millis(120);
    pub const POPUP_OUT: Duration = Duration::from_millis(80);

    pub fn ease_out_quint(t: f32) -> f32 {
        1.0 - (1.0 - t).powi(5)
    }
}

/// 浮いているもの（メニュー、パレット、パネル）に使う、ただ 1 種類の影。
pub fn popup_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow { color: hsla(0., 0., 0., 0.35), offset: point(px(0.), px(8.)), blur_radius: px(24.), spread_radius: px(0.), inset: false }]
}

pub fn ui_font() -> Font {
    let (family, fallbacks): (&str, &[&str]) = if cfg!(target_os = "macos") {
        (".SystemUIFont", &["Hiragino Sans"])
    } else {
        ("Segoe UI Variable Text", &["Segoe UI", "Yu Gothic UI"])
    };
    let mut f = font(family);
    f.fallbacks = Some(FontFallbacks::from_fonts(fallbacks.iter().map(|s| s.to_string()).collect()));
    f
}

pub fn mono_font() -> Font {
    font(MONO_FAMILY)
}

#[cfg(test)]
mod tests {
    use super::motion::ease_out_quint;

    #[test]
    fn ease_out_quint_endpoints() {
        assert_eq!(ease_out_quint(0.0), 0.0);
        assert_eq!(ease_out_quint(1.0), 1.0);
        assert!(ease_out_quint(0.5) > 0.9);
    }
}
