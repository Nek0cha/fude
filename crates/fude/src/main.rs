#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
mod hover;
mod main_area;
mod rail;
mod theme;
mod title_row;
mod workspace_view;

use std::borrow::Cow;
use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::assets::FudeAssets;
use crate::theme::Theme;
use crate::workspace_view::{WorkspaceView, bind_keys};

const MONO_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/HackGenConsoleNF-Regular.ttf");
const MONO_BOLD: &[u8] = include_bytes!("../../../assets/fonts/HackGenConsoleNF-Bold.ttf");

/// ウィンドウの背景の方式。Windows は MicaAlt（ユーザーが検証2の画面で選んだ見た目）、macOS は Blurred。
const WINDOW_BACKGROUND: WindowBackgroundAppearance = if cfg!(windows) {
    WindowBackgroundAppearance::MicaAltBackdrop
} else {
    WindowBackgroundAppearance::Blurred
};

/// 起動時に開くプロジェクト。引数の 1 つ目、無ければ現在のディレクトリ。
fn initial_root() -> PathBuf {
    let cwd = std::env::current_dir().expect("現在のディレクトリを取得できない");
    let root = match std::env::args_os().nth(1).map(PathBuf::from) {
        Some(arg) if arg.is_dir() => arg,
        Some(arg) => {
            eprintln!("ディレクトリではないため、現在のディレクトリを開く: {}", arg.display());
            cwd
        }
        None => cwd,
    };
    std::fs::canonicalize(&root).unwrap_or(root)
}

fn main() {
    let root = initial_root();
    application().with_assets(FudeAssets).run(move |cx: &mut App| {
        init(cx);
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(MONO_REGULAR), Cow::Borrowed(MONO_BOLD)])
            .expect("同梱の等幅フォントを読み込めない");
        cx.set_global(Theme::dark());
        bind_keys(cx);

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
        cx.open_window(options, move |window, cx| cx.new(|cx| WorkspaceView::new(root, window, cx)))
            .expect("ウィンドウを開けない");
        cx.activate(true);
    });
}
