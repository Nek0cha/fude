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
    /// 差分とツリーの状態の記号だけに使う「中身の色」（設計書 §6.1.1）。
    pub diff_add: Rgba,
    pub diff_del: Rgba,
    /// ターミナル内の検索の一致（warn の 30%）と、選んだ一致（warn の 70%）。
    pub search_match: Rgba,
    pub search_current: Rgba,
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
            diff_add: rgb(0x5fe0b0),
            diff_del: rgb(0xff8a8a),
            search_match: rgba(0xffd28a4d),
            search_current: rgba(0xffd28ab3),
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
    pub const PALETTE_W: Pixels = px(560.);
    pub const PALETTE_TOP: Pixels = px(72.);
    pub const PALETTE_SHIFT: Pixels = px(8.);
    pub const PALETTE_ROW_H: Pixels = px(30.);
    /// 返事待ちの枠の太さと、ウィンドウの角に合わせた丸み。
    pub const WAIT_FRAME_W: Pixels = px(1.5);
    pub const WINDOW_CORNER: Pixels = if cfg!(target_os = "macos") { px(10.) } else { px(8.) };
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
    /// コマンドパレット（§6.3）。出るときは上から PALETTE_SHIFT だけ下がりながら。
    pub const PALETTE_IN: Duration = Duration::from_millis(160);
    pub const PALETTE_OUT: Duration = Duration::from_millis(100);

    /// ホーム画面とプロジェクトの画面の切り替え。
    pub const SCREEN_FADE: Duration = Duration::from_millis(160);
    /// タブの追加（幅と文字）。
    pub const TAB_IN: Duration = Duration::from_millis(160);
    /// タブを閉じるときの、文字が消えるまでと、幅が縮むまで。
    pub const TAB_TEXT_OUT: Duration = Duration::from_millis(80);
    pub const TAB_OUT: Duration = Duration::from_millis(140);
    /// 分割の境目の移動。
    pub const SPLIT: Duration = Duration::from_millis(180);
    /// ターミナルの中身のフェードイン。
    pub const TERM_IN: Duration = Duration::from_millis(140);
    /// ターミナルが 0 枚の画面と並べた画面の間の、エディタの移動。
    pub const EDITOR_MOVE: Duration = Duration::from_millis(180);
    /// 右パネルと左パネルの出入り。
    pub const PANEL_IN: Duration = Duration::from_millis(180);
    pub const PANEL_OUT: Duration = Duration::from_millis(140);
    /// OS の「動きを減らす」がオンのときの、不透明度だけの変化。
    pub const REDUCED: Duration = Duration::from_millis(80);

    pub fn ease_out_quint(t: f32) -> f32 {
        1.0 - (1.0 - t).powi(5)
    }

    /// 返事待ちの枠が出るときと消えるとき（§6.3）。
    pub const WAIT_IN: Duration = Duration::from_millis(300);
    pub const WAIT_OUT: Duration = Duration::from_millis(200);
    /// 路線図の「実行中」の輪が次の駅へ動く時間（§6.3）。
    pub const RING_MOVE: Duration = Duration::from_millis(240);

    pub fn ease_in_out(t: f32) -> f32 {
        if t < 0.5 { 2.0 * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(2) / 2.0 }
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

/// GPUI Kit の部品（エディタ、差分）のテーマを fude に合わせる。背景は透明にし、ウィンドウの透過を残す。
pub fn apply_component_theme(cx: &mut gpui_kit::App) {
    use gpui_kit::component::{Theme as KitTheme, ThemeMode};
    let fude = *cx.global::<Theme>();
    KitTheme::change(ThemeMode::Dark, None, cx);
    let t = KitTheme::global_mut(cx);
    let clear = gpui_kit::hsla(0., 0., 0., 0.);
    t.mono_font_family = MONO_FAMILY.into();
    t.mono_font_size = size::TERM_FONT_SIZE;
    t.colors.background = clear;
    t.colors.foreground = fude.fg_default.into();
    t.colors.muted_foreground = fude.fg_dim.into();
    t.colors.border = fude.border_subtle.into();
    t.colors.input = fude.border_subtle.into();
    t.colors.popover = fude.bg_panel.into();
    t.colors.popover_foreground = fude.fg_default.into();
    t.colors.selection = fude.selection.into();
    t.colors.caret = fude.accent.into();
    t.colors.ring = fude.accent.into();
    t.colors.accent = fude.bg_hover.into();
    t.colors.list_hover = fude.bg_hover.into();
    let mut highlight = (*t.highlight_theme).clone();
    highlight.style.editor_background = Some(clear);
    highlight.style.editor_gutter_background = Some(clear);
    highlight.style.editor_active_line = Some(gpui_kit::hsla(0., 0., 1., 0.035));
    highlight.style.editor_foreground = Some(fude.fg_default.into());
    highlight.style.editor_line_number = Some(gpui_kit::Hsla::from(fude.fg_dim).opacity(0.6));
    highlight.style.editor_active_line_number = Some(fude.fg_dim.into());
    t.highlight_theme = std::sync::Arc::new(highlight);
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
