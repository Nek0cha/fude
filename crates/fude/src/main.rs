#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
mod theme;

use std::borrow::Cow;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::assets::FudeAssets;
use crate::theme::{Theme, ui_font};

const MONO_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/HackGenConsoleNF-Regular.ttf");
const MONO_BOLD: &[u8] = include_bytes!("../../../assets/fonts/HackGenConsoleNF-Bold.ttf");

/// ウィンドウの背景の方式。検証2の採用案（Windows は Mica、macOS は Blurred）。
const WINDOW_BACKGROUND: WindowBackgroundAppearance = if cfg!(windows) {
    WindowBackgroundAppearance::MicaBackdrop
} else {
    WindowBackgroundAppearance::Blurred
};

/// Task 6 で WorkspaceView に置き換える仮のビュー。
struct Placeholder;

impl Render for Placeholder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .size_full()
            .bg(theme.bg_window)
            .text_color(theme.fg_default)
            .font(ui_font())
            .p(px(24.))
            .child("fude")
    }
}

fn main() {
    application().with_assets(FudeAssets).run(|cx: &mut App| {
        init(cx);
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(MONO_REGULAR), Cow::Borrowed(MONO_BOLD)])
            .expect("同梱の等幅フォントを読み込めない");
        cx.set_global(Theme::dark());

        let base = TitleBar::window_options();
        let options = WindowOptions {
            titlebar: base.titlebar.map(|t| TitlebarOptions {
                traffic_light_position: Some(point(px(18.), px(13.))),
                ..t
            }),
            window_background: WINDOW_BACKGROUND,
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1200.), px(780.)), cx))),
            window_min_size: Some(size(px(640.), px(400.))),
            ..base
        };
        // gpui_kit::open_window は Root で包み、gpui-component のテーマの背景色（既定はライト）で
        // 全面を塗るため、ウィンドウの透過が見えなくなる。部品を使うまでは直接開く。
        cx.open_window(options, |_, cx| cx.new(|_| Placeholder)).expect("ウィンドウを開けない");
        cx.activate(true);
    });
}
