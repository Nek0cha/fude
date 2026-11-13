//! 色、寸法、動き、フォントのトークン（設計書 §6.2、§6.3）。
//! 描画コードには色や寸法の値を直接書かず、ここを通す。

use gpui_kit::{Font, FontFallbacks, Global, Rgba, font, rgb, rgba};

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
    /// Windows の閉じるボタンのホバー。OS の慣習に合わせる。
    pub window_close_hover: Rgba,
    /// プロジェクトのアイコンの色。明度と彩度を揃えた 6 色。
    pub project_colors: [Rgba; 6],
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
            window_close_hover: rgb(0xc42b1c),
            project_colors: [
                rgba(0x8fa7ff47),
                rgba(0x7fd1c747),
                rgba(0xe6b07a47),
                rgba(0xd99ad647),
                rgba(0x9fd38a47),
                rgba(0xe59a9a47),
            ],
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
    pub const TITLE_LEAD_W: Pixels = if cfg!(target_os = "macos") { px(80.) } else { RAIL_W };
    pub const TAB_H: Pixels = px(28.);
    pub const TAB_MAX_W: Pixels = px(200.);
    pub const CAPTION_BTN_W: Pixels = px(46.);
    pub const PROJECT_ICON: Pixels = px(32.);
    pub const RADIUS_SM: Pixels = px(6.);
    pub const RADIUS_MD: Pixels = px(8.);
}

/// 動きのトークン。
pub mod motion {
    use std::time::Duration;

    pub const HOVER_IN: Duration = Duration::from_millis(80);
    pub const HOVER_OUT: Duration = Duration::from_millis(120);

    pub fn ease_out(t: f32) -> f32 {
        1.0 - (1.0 - t) * (1.0 - t)
    }
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
