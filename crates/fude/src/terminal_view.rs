//! ペインごとのターミナルのビュー。セッション、フォーカス、入力、IME を受け持つ。

use std::cell::Cell;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use fude_core::keymap::Platform;
use fude_core::keys::{KeyOverrides, KeyRoute, RawKey, encode_key, route_key};
use fude_core::workspace::ShellKind;
use fude_term::{GridSize, SelectKind, Session, SessionEvent, WheelAccum, alt_scroll_bytes, paste_bytes, spawn_spec};
use futures::StreamExt;
use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::hover::{HoverFades, HoverKey};
use crate::terminal_paint::{CellMetrics, paint_terminal};
use crate::theme::size::{TERM_FONT_SIZE, TERM_PAD_X, TERM_PAD_Y};
use crate::theme::{Theme, mono_font};
use crate::title_row::icon;
use crate::workspace_view::{CopySelection, PasteText};

/// 「最新へ」のボタンの大きさと、ペインの端からの距離。
const TO_BOTTOM_SIZE: Pixels = px(28.);
const TO_BOTTOM_INSET: Pixels = px(12.);

/// ビューからワークスペースへ知らせること。
pub enum TerminalEvent {
    Title(Option<String>),
    Exited { success: bool },
    /// クリックでこのペインにフォーカスした。
    Focused,
}

pub struct TerminalView {
    pub(crate) session: Option<Arc<Session>>,
    /// 起動の失敗や異常終了のときに、ペインに出す文。
    message: Option<String>,
    focus: FocusHandle,
    /// IME の変換中の文字列。
    pub(crate) marked: Option<String>,
    pub(crate) metrics: CellMetrics,
    /// 升目の領域。描画のたびに更新する。
    pub(crate) bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    overrides: KeyOverrides,
    wheel: WheelAccum,
    hover: HoverFades,
    /// マウスで選択中なら Some（押してから動かしたか）。
    drag: Option<bool>,
    click_count: usize,
    _events: Task<()>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl TerminalView {
    pub fn new(kind: &ShellKind, root: &Path, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<SessionEvent>();
        let spec = spawn_spec(kind, root, &|k| std::env::var(k).ok());
        let on_event = Arc::new(move |ev| {
            let _ = tx.unbounded_send(ev);
        });
        let (session, message) = match Session::spawn(&spec, GridSize { cols: 80, rows: 24 }, on_event) {
            Ok(s) => (Some(Arc::new(s)), None),
            Err(err) => (None, Some(format!("シェルを起動できない: {err}"))),
        };
        // 溜まっている通知をまとめて読み、描き直しは 1 回にする（大量の出力で描画を詰まらせない）。
        let events = cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(ev) = rx.try_recv() {
                    batch.push(ev);
                }
                if this.update(cx, |this, cx| this.handle_events(batch, cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            session,
            message,
            focus: cx.focus_handle(),
            marked: None,
            metrics: CellMetrics::measure(window),
            bounds: Rc::new(Cell::new(None)),
            overrides: KeyOverrides::defaults(),
            wheel: WheelAccum::default(),
            hover: HoverFades::default(),
            drag: None,
            click_count: 0,
            _events: events,
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn handle_events(&mut self, events: Vec<SessionEvent>, cx: &mut Context<Self>) {
        for ev in events {
            match ev {
                SessionEvent::Wakeup => {}
                SessionEvent::Title(title) => cx.emit(TerminalEvent::Title(title)),
                SessionEvent::Exited { code } => {
                    let success = code == Some(0);
                    if !success {
                        let code = code.map_or_else(|| "不明".to_string(), |c| c.to_string());
                        self.message = Some(format!("プロセスが終了した（コード {code}）"));
                    }
                    cx.emit(TerminalEvent::Exited { success });
                }
            }
        }
        cx.notify();
    }

    /// 送る文字列を PTY に書く。
    pub(crate) fn write(&self, bytes: Vec<u8>) {
        if let Some(s) = &self.session {
            s.write(bytes);
        }
    }

    fn grid_size(&self) -> GridSize {
        self.session.as_ref().map_or(GridSize { cols: 80, rows: 24 }, |s| s.with_state(|st| st.size()))
    }

    /// ウィンドウの座標を、升目のセルにする。
    fn cell_at(&self, pos: Point<Pixels>) -> Option<(u16, u16, bool)> {
        let origin = self.bounds.get()?.origin;
        Some(point_to_cell(pos - origin, self.metrics, self.grid_size()))
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        // 変換中のキーは IME が処理する。
        if self.marked.is_some() {
            return;
        }
        let Some(session) = &self.session else { return };
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        let raw = RawKey { key: &ks.key, shift: m.shift, ctrl: m.control, alt: m.alt, platform: m.platform };
        if let KeyRoute::Encode(input) = route_key(&raw, Platform::current()) {
            let modes = session.with_state(|st| st.modes());
            if let Some(bytes) = encode_key(&input, modes, &self.overrides) {
                session.write(bytes);
            }
            cx.stop_propagation();
        }
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.emit(TerminalEvent::Focused);
        let (Some(session), Some((row, col, right))) = (&self.session, self.cell_at(ev.position)) else { return };
        let kind = match ev.click_count {
            2 => SelectKind::Word,
            n if n >= 3 => SelectKind::Line,
            _ => SelectKind::Simple,
        };
        session.with_state(|st| st.start_selection(row, col, right, kind));
        self.drag = Some(false);
        self.click_count = ev.click_count;
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.drag.is_none() || ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (Some(session), Some((row, col, right))) = (&self.session, self.cell_at(ev.position)) else { return };
        session.with_state(|st| st.update_selection(row, col, right));
        self.drag = Some(true);
        cx.notify();
    }

    fn on_mouse_up(&mut self, cx: &mut Context<Self>) {
        if let Some(moved) = self.drag.take() {
            if made_selection(moved, self.click_count) {
                // 選んだらすぐクリップボードへ入れる（ユーザーの決定）。
                self.copy(cx);
            } else if let Some(s) = &self.session {
                // 押しただけで動かさなかったクリックは、選択を残さない。
                s.with_state(|st| st.clear_selection());
            }
        }
        cx.notify();
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return };
        let lines = match ev.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => p.y / self.metrics.line_h,
        };
        let n = self.wheel.feed(lines);
        if n == 0 {
            return;
        }
        let alt = session.with_state(|st| {
            if st.alt_scroll() {
                Some(st.modes().app_cursor)
            } else {
                st.scroll(n);
                None
            }
        });
        if let Some(app_cursor) = alt {
            session.write(alt_scroll_bytes(n, app_cursor));
        }
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.session.as_ref().and_then(|s| s.with_state(|st| st.selection_text())) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let (Some(session), Some(text)) = (&self.session, cx.read_from_clipboard().and_then(|c| c.text())) else {
            return;
        };
        let bracketed = session.with_state(|st| st.bracketed_paste());
        session.write(paste_bytes(&text, bracketed));
    }

    /// 上へ遡っているときだけ出す「最新へ」のボタン。
    fn render_to_bottom(&mut self, theme: &Theme, now: Instant, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let offset = self.session.as_ref().map_or(0, |s| s.with_state(|st| st.display_offset()));
        if offset == 0 {
            return None;
        }
        let key = HoverKey::ScrollToBottom;
        let hover = Rgba { a: theme.bg_hover.a * self.hover.value(key, now), ..theme.bg_hover };
        let face = div()
            .size_full()
            .rounded_full()
            .bg(hover)
            .flex()
            .items_center()
            .justify_center()
            .child(icon(IconName::ArrowDown, theme.fg_default));
        Some(
            div()
                .id("to-bottom")
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.hover.set(key, *hovered, Instant::now());
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(s) = &this.session {
                        s.with_state(|st| st.scroll_to_bottom());
                    }
                    cx.notify();
                }))
                .absolute()
                .right(TO_BOTTOM_INSET)
                .bottom(TO_BOTTOM_INSET)
                .size(TO_BOTTOM_SIZE)
                .rounded_full()
                .bg(theme.bg_panel)
                .border_1()
                .border_color(theme.border_subtle)
                .child(face),
        )
    }

    /// カーソルのセルの四角（ウィンドウの座標）。IME の候補ウィンドウの位置に使う。
    fn cursor_bounds(&self, cx: &App) -> Option<Bounds<Pixels>> {
        let origin = self.bounds.get()?.origin;
        let palette = cx.global::<Theme>().term_palette;
        let cursor = self.session.as_ref()?.with_state(|st| st.snapshot(&palette, true).cursor)?;
        let m = self.metrics;
        let at = point(origin.x + m.cell_w * f32::from(cursor.col), origin.y + m.line_h * f32::from(cursor.row));
        Some(Bounds::new(at, size(m.cell_w, m.line_h)))
    }
}

/// マウスを離した時点で、選択ができているか（ドラッグしたか、ダブル・トリプルクリックか）。
pub(crate) fn made_selection(moved: bool, click_count: usize) -> bool {
    moved || click_count >= 2
}

/// 要素の中の位置（パディングを引いたあと）をセルにする。範囲外は端に寄せる。
/// 戻り値は (行, 列, セルの右半分か)。
pub(crate) fn point_to_cell(pos: Point<Pixels>, m: CellMetrics, size: GridSize) -> (u16, u16, bool) {
    let x = pos.x.max(px(0.)) / m.cell_w;
    let y = pos.y.max(px(0.)) / m.line_h;
    let col = (x.floor() as u16).min(size.cols.saturating_sub(1));
    let row = (y.floor() as u16).min(size.rows.saturating_sub(1));
    let right_half = if x >= f32::from(size.cols) { true } else { x.fract() >= 0.5 };
    (row, col, right_half)
}

impl EntityInputHandler for TerminalView {
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
        let end = self.marked.as_deref().map_or(0, |m| m.encode_utf16().count());
        Some(UTF16Selection { range: end..end, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_deref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        if !text.is_empty() {
            self.write(text.as_bytes().to_vec());
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
        _: Bounds<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds(cx)
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let now = Instant::now();
        self.hover.set_window_hovered(window.is_window_hovered(), now);
        if self.hover.any_animating(now) {
            window.request_animation_frame();
        }
        let to_bottom = self.render_to_bottom(&theme, now, cx);
        let focused = self.focus.is_focused(window);
        let entity = cx.entity();
        let focus = self.focus.clone();
        let session = self.session.clone();
        let marked = self.marked.clone();
        let metrics = self.metrics;
        let bounds_cell = self.bounds.clone();

        let grid = canvas(
            move |bounds, _, _| {
                bounds_cell.set(Some(bounds));
                if let Some(s) = &session {
                    let w = f32::from(bounds.size.width);
                    let h = f32::from(bounds.size.height);
                    s.resize(GridSize::from_pixels(w, h, f32::from(metrics.cell_w), f32::from(metrics.line_h)));
                }
                session
            },
            move |bounds, session, window, cx| {
                if let Some(s) = session {
                    let snap = s.with_state(|st| st.snapshot(&theme.term_palette, focused));
                    paint_terminal(&snap, marked.as_deref(), bounds.origin, metrics, &theme, window, cx);
                }
                window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
            },
        )
        .size_full();

        let msg_color = if self.session.is_some() { theme.fg_dim } else { theme.danger };
        let message = self.message.clone().map(|msg| {
            div().absolute().left(TERM_PAD_X).bottom(TERM_PAD_Y).text_color(msg_color).child(msg)
        });
        div()
            .id("terminal")
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key(ev, cx)))
            .on_action(cx.listener(|this, _: &CopySelection, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &PasteText, _, cx| this.paste(cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| this.on_mouse_down(ev, window, cx)),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.on_mouse_move(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, cx| this.on_mouse_up(cx)))
            .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| this.on_scroll(ev, cx)))
            .relative()
            .size_full()
            .px(TERM_PAD_X)
            .py(TERM_PAD_Y)
            .font(mono_font())
            .text_size(TERM_FONT_SIZE)
            .child(grid)
            .children(message)
            .children(to_bottom)
    }
}

#[cfg(test)]
mod tests {
    // gpui_kit の glob は test 属性マクロを含むため、必要な名前だけを取り込む。
    use super::{CellMetrics, GridSize, made_selection, point_to_cell};
    use gpui_kit::{point, px};

    fn metrics() -> CellMetrics {
        CellMetrics { cell_w: px(8.), line_h: px(18.) }
    }

    const SIZE: GridSize = GridSize { cols: 80, rows: 24 };

    #[test]
    fn point_to_cell_basic() {
        assert_eq!(point_to_cell(point(px(20.), px(40.)), metrics(), SIZE), (2, 2, true));
        assert_eq!(point_to_cell(point(px(3.), px(1.)), metrics(), SIZE), (0, 0, false));
    }

    /// ドラッグか、ダブル・トリプルクリックで選んだときだけ、離した時点でコピーする。
    #[test]
    fn copy_on_release_only_after_selecting() {
        assert!(!made_selection(false, 1));
        assert!(made_selection(true, 1));
        assert!(made_selection(false, 2));
        assert!(made_selection(false, 3));
    }

    #[test]
    fn point_to_cell_clamps() {
        assert_eq!(point_to_cell(point(px(-5.), px(-5.)), metrics(), SIZE), (0, 0, false));
        assert_eq!(point_to_cell(point(px(10000.), px(10000.)), metrics(), SIZE), (23, 79, true));
    }
}
