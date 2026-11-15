//! 色、寸法、動き、フォントのトークン（設計書 §6.2、§6.3）。
//! 描画コードには色や寸法の値を直接書かず、ここを通す。

use fude_term::Palette;
use fude_core::settings::{FontSettings, Settings};
use gpui_kit::{BoxShadow, Font, FontFallbacks, Global, Pixels, Rgba, font, hsla, point, px};

/// ターミナルとエディタの書体と大きさ（設定の [font]）。描画のたびに読むので、Global ではなく静的な値に置く
/// （mono_font() などは cx を受けない箇所からも呼ばれるため）。None は既定値。
static FONTS: std::sync::RwLock<Option<FontSettings>> = std::sync::RwLock::new(None);

fn fonts() -> FontSettings {
    FONTS.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| Settings::default().font)
}

/// 設定の [font] を反映する。描き直しは呼び出し側が行う。
pub fn set_fonts(font: &FontSettings) {
    *FONTS.write().unwrap_or_else(|e| e.into_inner()) = Some(font.clone());
}

/// ターミナルとエディタの文字の大きさ。
pub fn term_font_size() -> Pixels {
    px(fonts().size)
}

/// ターミナルとエディタの行の高さ。
pub fn term_line_height() -> Pixels {
    px(fonts().line_height)
}

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
    /// 構文ハイライトの色。None なら GPUI Kit の既定のまま。
    pub syntax: Option<crate::themes::SyntaxPalette>,
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

/// UI の書体。設定の [font] ui を書けばそれを先に使い、無い文字は OS の既定の書体で補う。
pub fn ui_font() -> Font {
    let (family, fallbacks): (&str, &[&str]) = if cfg!(target_os = "macos") {
        (".SystemUIFont", &["Hiragino Sans"])
    } else {
        ("Segoe UI Variable Text", &["Segoe UI", "Yu Gothic UI"])
    };
    let mut fallbacks: Vec<String> = fallbacks.iter().map(|s| s.to_string()).collect();
    let family = match fonts().ui {
        Some(custom) => {
            fallbacks.insert(0, family.to_string());
            custom
        }
        None => family.to_string(),
    };
    let mut f = font(family);
    f.fallbacks = Some(FontFallbacks::from_fonts(fallbacks));
    f
}

pub fn mono_font() -> Font {
    font(fonts().mono)
}

/// fude の構文の色を GPUI Kit の SyntaxColors にする。ThemeStyle のフィールドが非公開なので JSON から作る。
pub fn kit_syntax(p: &crate::themes::SyntaxPalette) -> gpui_kit::component::highlighter::SyntaxColors {
    let c = |v: u32| serde_json::json!({ "color": format!("#{v:06x}") });
    let value = serde_json::json!({
        "attribute": c(p.attribute),
        "boolean": c(p.constant),
        "comment": c(p.comment),
        "comment_doc": c(p.comment),
        "constant": c(p.constant),
        "constructor": c(p.type_),
        "enum": c(p.type_),
        "function": c(p.function),
        "keyword": c(p.keyword),
        "label": c(p.constant),
        "link_text": c(p.link),
        "link_uri": c(p.link),
        "number": c(p.number),
        "operator": c(p.operator),
        "property": c(p.property),
        "punctuation": c(p.punctuation),
        "punctuation.bracket": c(p.punctuation),
        "punctuation.delimiter": c(p.punctuation),
        "punctuation.special": c(p.operator),
        "string": c(p.string),
        "string.escape": c(p.operator),
        "string.special": c(p.string),
        "tag": c(p.tag),
        "title": c(p.title),
        "type": c(p.type_),
        "variable": c(p.variable),
        "variable.special": c(p.keyword),
        "variant": c(p.constant),
    });
    serde_json::from_value(value).expect("構文の色の JSON の形")
}

/// GPUI Kit の部品（エディタ、差分）のテーマを fude に合わせる。背景は透明にし、ウィンドウの透過を残す。
pub fn apply_component_theme(cx: &mut gpui_kit::App) {
    use gpui_kit::component::{Theme as KitTheme, ThemeMode};
    let fude = *cx.global::<Theme>();
    KitTheme::change(ThemeMode::Dark, None, cx);
    let t = KitTheme::global_mut(cx);
    let clear = gpui_kit::hsla(0., 0., 0., 0.);
    let fonts = fonts();
    t.mono_font_family = fonts.mono.into();
    t.mono_font_size = px(fonts.size);
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
    if let Some(syntax) = &fude.syntax {
        highlight.style.syntax = kit_syntax(syntax);
    }
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

    #[test]
    fn kit_syntax_carries_colors() {
        use super::kit_syntax;
        let p = crate::themes::colors(fude_core::settings::ThemeId::Dracula).syntax.unwrap();
        let v = serde_json::to_value(kit_syntax(&p)).unwrap();
        assert_eq!(v["keyword"]["color"].as_str().map(str::to_lowercase).as_deref(), Some("#ff79c6ff"));
        assert!(v["type"]["color"].is_string());
    }

    #[test]
    fn term_font_size_follows_settings() {
        use super::{set_fonts, term_font_size, term_line_height};
        use fude_core::settings::Settings;
        use gpui_kit::px;
        let mut font = Settings::default().font;
        font.size = 18.0;
        font.line_height = 24.0;
        set_fonts(&font);
        assert_eq!(term_font_size(), px(18.));
        assert_eq!(term_line_height(), px(24.));
        set_fonts(&Settings::default().font);
        assert_eq!(term_font_size(), px(14.));
        assert_eq!(term_line_height(), px(18.));
    }
}
