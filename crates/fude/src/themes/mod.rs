//! 同梱するテーマ（暗い版だけ。設計書 `2026-11-15-config-theme-design.md` §5.1、§5.2）。
//! 各テーマは不透明の色だけを持ち、透け具合は build_theme が設定の background_opacity から掛ける。
//! 配色の出典とライセンスは licenses/ の各ファイル。

mod catppuccin;
mod dracula;
mod fude;
mod github;
mod one_dark_pro;
mod tokyo_night;

use fude_core::settings::{Settings, ThemeId};
use fude_term::Palette;
use gpui_kit::{Rgba, rgb, rgba};

use crate::theme::Theme;

/// 構文ハイライトの色（0xrrggbb）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxPalette {
    pub keyword: u32,
    pub string: u32,
    pub comment: u32,
    pub function: u32,
    pub type_: u32,
    pub constant: u32,
    pub number: u32,
    pub variable: u32,
    pub property: u32,
    pub operator: u32,
    pub punctuation: u32,
    pub tag: u32,
    pub attribute: u32,
    pub title: u32,
    pub link: u32,
}

/// テーマの色の組。すべて不透明の 0xrrggbb。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeColors {
    pub bg_window: u32,
    pub bg_panel: u32,
    pub fg_default: u32,
    pub fg_dim: u32,
    pub accent: u32,
    pub warn: u32,
    pub danger: u32,
    /// プロジェクトのアイコンの色。build_theme が 28% の濃さにする。
    pub project_colors: [u32; 6],
    pub diff_add: u32,
    pub diff_del: u32,
    pub term: Palette,
    /// None なら GPUI Kit の既定の構文の色のまま（fude と oled）。
    pub syntax: Option<SyntaxPalette>,
}

pub fn colors(id: ThemeId) -> ThemeColors {
    match id {
        ThemeId::Fude => fude::FUDE,
        ThemeId::Oled => fude::OLED,
        ThemeId::GithubDark => github::DARK,
        ThemeId::GithubDarkDimmed => github::DARK_DIMMED,
        ThemeId::OneDarkPro => one_dark_pro::ONE_DARK_PRO,
        ThemeId::Dracula => dracula::DRACULA,
        ThemeId::TokyoNight => tokyo_night::NIGHT,
        ThemeId::TokyoNightStorm => tokyo_night::STORM,
        ThemeId::CatppuccinMocha => catppuccin::MOCHA,
        ThemeId::CatppuccinMacchiato => catppuccin::MACCHIATO,
        ThemeId::CatppuccinFrappe => catppuccin::FRAPPE,
    }
}

/// パネルや浮かぶものの、今の濃さ（0xeb）。地より薄くはしない。
const PANEL_ALPHA: u32 = 0xeb;

fn with_alpha(color: u32, alpha: u32) -> Rgba {
    rgba((color << 8) | alpha.min(0xff))
}

/// 0.0〜1.0 の不透明度を 0〜255 にする。
fn alpha_of(opacity: f32) -> u32 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u32
}

/// 設定（テーマ、アクセントの上書き、背景の不透明度）から Theme を組み立てる。
pub fn build_theme(settings: &Settings) -> Theme {
    let c = colors(settings.theme);
    let accent = settings.accent.unwrap_or(c.accent);
    let window_alpha = alpha_of(settings.window.background_opacity);
    let panel_alpha = window_alpha.max(PANEL_ALPHA);
    let mut term = c.term;
    if settings.accent.is_some() {
        term.cursor = accent;
    }
    Theme {
        bg_window: with_alpha(c.bg_window, window_alpha),
        bg_panel: with_alpha(c.bg_panel, panel_alpha),
        bg_hover: rgba(0xffffff12),
        border_subtle: rgba(0xffffff1f),
        fg_default: rgb(c.fg_default),
        fg_dim: rgb(c.fg_dim),
        accent: rgb(accent),
        warn: rgb(c.warn),
        danger: rgb(c.danger),
        window_close_hover: with_alpha(c.danger, 0x38),
        project_colors: c.project_colors.map(|p| with_alpha(p, 0x47)),
        term_palette: term,
        selection: with_alpha(accent, 0x4d),
        diff_add: rgb(c.diff_add),
        diff_del: rgb(c.diff_del),
        search_match: with_alpha(c.warn, 0x4d),
        search_current: with_alpha(c.warn, 0xb3),
        syntax: c.syntax,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fude_core::settings::{Settings, ThemeId};

    #[test]
    fn every_theme_has_colors() {
        for id in ThemeId::ALL {
            let c = colors(id);
            assert_ne!(c.bg_window, c.fg_default, "{id:?}");
            assert_ne!(c.bg_window, c.fg_dim, "{id:?}");
            assert_eq!(c.term.background, c.bg_window, "{id:?}: 反転の背景は地の色");
        }
    }

    #[test]
    fn fude_theme_matches_old_dark() {
        let t = build_theme(&Settings::default());
        assert_eq!(t.bg_window, rgba(0x0e0e149e));
        assert_eq!(t.bg_panel, rgba(0x1e1e2aeb));
        assert_eq!(t.accent, rgb(0xb9a8ff));
        assert_eq!(t.selection, rgba(0xb9a8ff4d));
        assert_eq!(t.window_close_hover, rgba(0xff8a8a38));
        assert_eq!(t.project_colors[0], rgba(0x8fa7ff47));
        assert_eq!(t.search_current, rgba(0xffd28ab3));
        assert_eq!(
            t.term_palette.ansi,
            [
                0x1e1e2a, 0xff8a8a, 0x5fe0b0, 0xffd28a, 0x8fa7ff, 0xd99ad6, 0x7fd1c7, 0xd8d8e2, //
                0x7b7d90, 0xffa3a3, 0x84ecc4, 0xffe0a8, 0xaabcff, 0xe8b4e6, 0x9fe3da, 0xf2f2f7,
            ]
        );
        assert_eq!(t.term_palette.cursor, 0xb9a8ff);
        assert!(t.syntax.is_none());
    }

    #[test]
    fn opaque_setting_makes_window_opaque() {
        let mut s = Settings::default();
        s.window.background_opacity = 1.0;
        let t = build_theme(&s);
        assert_eq!(t.bg_window.a, 1.0);
        assert_eq!(t.bg_panel.a, 1.0);
    }

    #[test]
    fn panel_is_never_thinner_than_now() {
        let mut s = Settings::default();
        s.window.background_opacity = 0.3;
        let t = build_theme(&s);
        assert!((t.bg_panel.a - 0xeb as f32 / 255.0).abs() < 1e-6);
        assert!((t.bg_window.a - 0.3).abs() < 0.01);
    }

    #[test]
    fn accent_override_wins() {
        let s = Settings { theme: ThemeId::Dracula, accent: Some(0xff0080), ..Settings::default() };
        let t = build_theme(&s);
        assert_eq!(t.accent, rgb(0xff0080));
        assert_eq!(t.selection, rgba(0xff00804d));
        assert_eq!(t.term_palette.cursor, 0xff0080);
    }

    #[test]
    fn oled_is_black() {
        assert_eq!(colors(ThemeId::Oled).bg_window, 0x000000);
        assert_eq!(colors(ThemeId::Oled).term.background, 0x000000);
    }

    #[test]
    fn named_themes_have_syntax_colors() {
        for id in ThemeId::ALL.into_iter().filter(|id| !matches!(id, ThemeId::Fude | ThemeId::Oled)) {
            assert!(colors(id).syntax.is_some(), "{id:?}");
        }
    }
}
