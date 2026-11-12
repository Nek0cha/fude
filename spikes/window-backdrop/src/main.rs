//! 検証2：ウィンドウの背景の方式（透過、ぼかし、Mica）と、HackGen の表示を確かめる。
//! スペースキーで背景の方式を順に切り替える。

use std::borrow::Cow;

use gpui_kit::*;

const MONO_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/HackGenConsoleNF-Regular.ttf");
const MONO_BOLD: &[u8] = include_bytes!("../../../assets/fonts/HackGenConsoleNF-Bold.ttf");
const MONO_FAMILY: &str = "HackGen Console NF";

/// gpui-pre 0.3.12 の WindowBackgroundAppearance の列挙値すべて。
const MODES: [WindowBackgroundAppearance; 5] = [
    WindowBackgroundAppearance::Opaque,
    WindowBackgroundAppearance::Transparent,
    WindowBackgroundAppearance::Blurred,
    WindowBackgroundAppearance::MicaBackdrop,
    WindowBackgroundAppearance::MicaAltBackdrop,
];

const TERMINAL_LINES: [&str; 6] = [
    "~/fude ❯ claude",
    "› WSL でもシェルが起動するようにして",
    "● Edited src/terminal/pty.rs  +24 −6",
    "  // Windows は ConPTY、macOS は openpty",
    "  \u{f418} main   \u{e7a8} rust 1.99.1",
    "ああああああああああ|",
];
/// 上の最終行と同じ幅になるはずの半角 20 文字。桁のずれを目で見るために並べる。
const ALIGN_LINE: &str = "aaaaaaaaaaaaaaaaaaaa|";

struct Backdrop {
    mode: usize,
    focus: FocusHandle,
}

impl Render for Backdrop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.focus(&self.focus, cx);
        let mono = |text: &'static str| div().font_family(MONO_FAMILY).child(text);
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "space" {
                    this.mode = (this.mode + 1) % MODES.len();
                    window.set_background_appearance(MODES[this.mode]);
                    cx.notify();
                }
            }))
            .size_full()
            .relative()
            .bg(rgba(0x0e0e149e))
            .text_color(rgb(0xd8d8e2))
            .text_size(px(14.))
            .pt(px(48.))
            .px(px(24.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .absolute()
                    .top(px(12.))
                    .right(px(16.))
                    .text_size(px(12.))
                    .child(format!("{:?}（スペースで切り替え）", MODES[self.mode])),
            )
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).mb(px(12.)).child("外観"))
            .children(TERMINAL_LINES.map(mono))
            .child(mono(ALIGN_LINE))
            .child(div().font_family(MONO_FAMILY).font_weight(FontWeight::BOLD).child("Bold: 太字の確認 0123"))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(MONO_REGULAR), Cow::Borrowed(MONO_BOLD)])
            .expect("フォントを読み込めない");

        let font_id = cx.text_system().resolve_font(&font(MONO_FAMILY));
        let font_size = px(14.);
        let ascii = cx.text_system().advance(font_id, font_size, 'a').unwrap().width;
        let cjk = cx.text_system().advance(font_id, font_size, 'あ').unwrap().width;
        println!("ascii advance: {ascii:?}, cjk advance: {cjk:?}");
        println!("CJK width ratio: {:.3}", cjk / ascii);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(900.), px(560.)), cx))),
            titlebar: Some(TitlebarOptions {
                title: Some("window-backdrop".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(12.), px(12.))),
            }),
            window_background: MODES[0],
            ..Default::default()
        };
        cx.open_window(options, |_, cx| cx.new(|cx| Backdrop { mode: 0, focus: cx.focus_handle() }))
            .expect("ウィンドウを開けない");
        cx.activate(true);
    });
}
