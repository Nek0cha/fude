#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod anim;
mod attention;
mod assets;
mod changes_menu;
mod cli;
mod close_flow;
mod columns;
mod dialog;
mod drag;
mod editor_column;
mod editor_tabs_view;
mod file_icons;
mod file_view;
mod home;
mod hook_cli;
mod hover;
mod instance;
mod language;
mod layout_anim;
mod main_area;
mod md_preview;
mod open_file;
mod palette_view;
mod pin_view;
mod popup;
mod project_state;
mod project_menu;
mod rail;
mod route_map;
mod search_view;
mod reduced_motion;
mod shell_menu;
mod term_search;
mod terminal_paint;
mod terminal_view;
mod theme;
mod title_row;
mod tree_menu;
mod tree_model;
mod tree_view;
mod workspace_view;
mod wsl_host;

use std::borrow::Cow;

use fude_core::keymap::Platform;
use fude_core::recent::{self, RecentList};
use fude_core::session::{self, WindowGeom, window_is_visible};
use fude_ipc::Message;
use futures::StreamExt;
use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::assets::AppAssets;
use crate::cli::{Launch, parse_launch};
use crate::instance::{HandOff, hand_off, report_error};
use crate::theme::Theme;
use crate::workspace_view::{InitialOpen, Startup, WorkspaceView, bind_keys};

const MONO_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/HackGenConsoleNF-Regular.ttf");
const MONO_BOLD: &[u8] = include_bytes!("../../../assets/fonts/HackGenConsoleNF-Bold.ttf");

/// ウィンドウの背景の方式。Windows と macOS のどちらも Blurred（ユーザーが検証2の画面で選んだ見た目）。
const WINDOW_BACKGROUND: WindowBackgroundAppearance = WindowBackgroundAppearance::Blurred;

/// 保存した位置がどれかのディスプレイに見えていればそこへ、そうでなければ画面の中央に 1200×780 で開く。
fn initial_bounds(saved: Option<WindowGeom>, cx: &App) -> WindowBounds {
    let displays: Vec<(f32, f32, f32, f32)> = cx
        .displays()
        .iter()
        .map(|d| {
            let b = d.bounds();
            (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height))
        })
        .collect();
    match saved.filter(|g| window_is_visible(g, &displays)) {
        Some(g) => {
            let bounds = Bounds::new(point(px(g.x), px(g.y)), size(px(g.width), px(g.height)));
            if g.maximized { WindowBounds::Maximized(bounds) } else { WindowBounds::Windowed(bounds) }
        }
        None => WindowBounds::Windowed(Bounds::centered(None, size(px(1200.), px(780.)), cx)),
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    // `fude hook` は待ち受けも GUI も始めず、信号を送って終わる（`hook` というフォルダは `fude ./hook` で開く）。
    if args.first().is_some_and(|a| a == "hook") {
        std::process::exit(hook_cli::hook_main(&args[1..]));
    }
    let cwd = std::env::current_dir().expect("現在のディレクトリを取得できない");
    let launch = match parse_launch(&args, &cwd) {
        Ok(launch) => launch,
        Err(e) => {
            report_error(&e);
            std::process::exit(2);
        }
    };
    let name = fude_ipc::endpoint_name(&|k| std::env::var(k).ok());
    let server = match hand_off(&name, &launch) {
        Ok(HandOff::Delivered) => return,
        Ok(HandOff::Serve(server)) => server,
        Err(e) => {
            report_error(&e);
            std::process::exit(1);
        }
    };
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<Message>();
    server.serve(move |m| {
        tx.unbounded_send(m).ok();
    });

    // 前回の状態は自動では戻さない。ウィンドウの位置と大きさだけを使い、残りはホーム画面の「前回の続き」で戻す。
    let env = |k: &str| std::env::var(k).ok();
    let session_path = session::session_path(Platform::current(), &env);
    let saved = session_path.as_deref().and_then(session::load);
    let saved_window = saved.as_ref().and_then(|d| d.window.clone());
    let recent_path = recent::recent_path(Platform::current(), &env);
    let recent = recent_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| RecentList::parse(&text))
        .unwrap_or_default();
    let open = match launch {
        Launch::Open { root, shell } => InitialOpen::Project { root, shell },
        Launch::OpenFile { file, root_hint } => InitialOpen::File { file, root_hint },
        Launch::Activate => InitialOpen::Nothing,
    };
    let startup = Startup { session_path, last_session: saved.filter(|d| d.has_projects()), recent_path, recent, open };

    application().with_assets(AppAssets).run(move |cx: &mut App| {
        init(cx);
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(MONO_REGULAR), Cow::Borrowed(MONO_BOLD)])
            .expect("同梱の等幅フォントを読み込めない");
        cx.set_global(Theme::dark());
        theme::apply_component_theme(cx);
        bind_keys(cx);
        file_view::bind_file_view_keys(cx);
        tree_view::bind_tree_keys(cx);
        palette_view::bind_palette_keys(cx);
        term_search::bind_term_search_keys(cx);
        search_view::bind_search_view_keys(cx);
        // ウィンドウは 1 枚なので、閉じたらアプリも終える（macOS でアプリだけ残らないように）。
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let base = TitleBar::window_options();
        let options = WindowOptions {
            titlebar: base.titlebar.map(|t| TitlebarOptions {
                traffic_light_position: Some(point(px(18.), px(13.))),
                ..t
            }),
            window_background: WINDOW_BACKGROUND,
            window_bounds: Some(initial_bounds(saved_window, cx)),
            window_min_size: Some(size(px(640.), px(400.))),
            ..base
        };
        // gpui_kit::open_window は Root で包み、gpui-component のテーマの背景色（既定はライト）で
        // 全面を塗るため、ウィンドウの透過が見えなくなる。部品を使うまでは直接開く。
        let handle = cx
            .open_window(options, move |window, cx| cx.new(|cx| WorkspaceView::new(startup, window, cx)))
            .expect("ウィンドウを開けない");
        // 2 つ目の起動から届いた要求を、ウィンドウのビューで処理する。
        handle
            .update(cx, |_, window, cx| {
                cx.spawn_in(window, async move |this, cx| {
                    while let Some(msg) = rx.next().await {
                        if this.update_in(cx, |view, window, cx| view.handle_message(msg, window, cx)).is_err() {
                            break;
                        }
                    }
                })
                .detach();
            })
            .expect("ウィンドウを更新できない");
        cx.activate(true);
    });
}
