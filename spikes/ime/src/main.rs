//! 検証3：日本語 IME の入力を確かめる。
//! 上半分は、テキスト欄を持たない自前のビュー（ターミナルのビューの代わり）。
//! 下半分は、GPUI Kit の Editor（Markdown）。

use std::borrow::Cow;
use std::ops::Range;

use fude_core::keys::{KeyCode, KeyInput, KeyOverrides, Modifiers, TermModes, encode_key};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::*;

const MONO_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/HackGen35ConsoleNF-Regular.ttf");
const MONO_FAMILY: &str = "HackGen35 Console NF";
const LINE_HEIGHT: f32 = 22.;
const PADDING: f32 = 16.;
/// 画面に残す確定済みの行数。
const KEEP_LINES: usize = 6;

struct ImeProbe {
    focus: FocusHandle,
    committed: Vec<String>,
    marked: Option<String>,
}

impl ImeProbe {
    /// 擬似カーソルの、ビューの左上からの位置。最後の行の先頭に置く。
    fn cursor_offset(&self) -> Point<Pixels> {
        let row = self.committed.len().min(KEEP_LINES) as f32;
        point(px(PADDING + 16.), px(PADDING + LINE_HEIGHT * row))
    }

    fn marked_len_utf16(&self) -> usize {
        self.marked.as_deref().map_or(0, |m| m.encode_utf16().count())
    }
}

/// 確定した文字列を、ターミナルに送る場合のバイト列（16 進数）にする。
fn to_hex(text: &str) -> String {
    let overrides = KeyOverrides::defaults();
    text.chars()
        .filter_map(|c| {
            let key = KeyInput { code: KeyCode::Char(c), mods: Modifiers::default() };
            encode_key(&key, TermModes::default(), &overrides)
        })
        .flatten()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl EntityInputHandler for ImeProbe {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let marked: Vec<u16> = self.marked.as_deref().unwrap_or("").encode_utf16().collect();
        let end = range.end.min(marked.len());
        let start = range.start.min(end);
        *adjusted_range = Some(start..end);
        Some(String::from_utf16_lossy(&marked[start..end]))
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let end = self.marked_len_utf16();
        Some(UTF16Selection { range: end..end, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|_| 0..self.marked_len_utf16())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        if !text.is_empty() {
            println!("[commit] {text}  ->  {}", to_hex(text));
            self.committed.push(text.to_string());
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let origin = element_bounds.origin + self.cursor_offset();
        Some(Bounds::new(origin, size(px(2.), px(LINE_HEIGHT))))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

impl Render for ImeProbe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let focus = self.focus.clone();
        let shown = self.committed.iter().rev().take(KEEP_LINES).rev().cloned();
        div()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
                // 変換中のキーは IME が処理する。ここでターミナルに送ってはいけない。
                if this.marked.is_some() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "backspace" => println!("[key] backspace"),
                    "enter" => println!("[key] enter"),
                    _ => {}
                }
            }))
            .relative()
            .h_1_2()
            .w_full()
            .p(px(PADDING))
            .font_family(MONO_FAMILY)
            .text_size(px(14.))
            .line_height(px(LINE_HEIGHT))
            .bg(rgb(0x14141c))
            .text_color(rgb(0xd8d8e2))
            .children(shown.map(|line| div().child(format!("  {line}"))))
            .child(
                div()
                    .flex()
                    .child("› ")
                    .child(
                        div()
                            .underline()
                            .text_color(rgb(0xb9a8ff))
                            .child(self.marked.clone().unwrap_or_default()),
                    )
                    .child(div().w(px(8.)).h(px(16.)).bg(rgb(0xb9a8ff))),
            )
            .child(
                canvas(|_, _, _| {}, move |bounds, _, window, cx| {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                })
                .absolute()
                .size_full(),
            )
    }
}

struct Root2 {
    probe: Entity<ImeProbe>,
    editor: Entity<EditorState>,
    /// Editor は作成時にフォーカスを取るため、最初の描画で上のビューへ移す。
    focused_once: bool,
}

impl Render for Root2 {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            let focus = self.probe.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.probe.clone())
            .child(div().h_1_2().p(px(8.)).child(Editor::new(&self.editor).h_full()))
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx: &mut App| {
        init(cx);
        cx.text_system().add_fonts(vec![Cow::Borrowed(MONO_REGULAR)]).expect("フォントを読み込めない");
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(820.), px(620.)), cx))),
            ..Default::default()
        };
        open_window(options, cx, |window, cx| {
            let probe = cx.new(|cx| ImeProbe { focus: cx.focus_handle(), committed: Vec::new(), marked: None });
            let editor = cx.new(|cx| {
                EditorState::new(window, cx).language("markdown").default_value("# 見出し\n\nここで日本語を入力する。\n")
            });
            cx.new(|_| Root2 { probe, editor, focused_once: false })
        })
        .expect("ウィンドウを開けない");
        cx.activate(true);
    });
}
