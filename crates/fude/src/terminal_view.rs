//! ペインごとのターミナルのビュー。セッション、フォーカス、入力、IME を受け持つ。

use std::cell::Cell;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use fude_core::workspace::ShellKind;
use fude_term::{GridSize, Session, SessionEvent, spawn_spec};
use futures::StreamExt;
use gpui_kit::*;

use crate::terminal_paint::{CellMetrics, paint_terminal};
use crate::theme::size::{TERM_FONT_SIZE, TERM_PAD_X, TERM_PAD_Y};
use crate::theme::{Theme, mono_font};

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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    cx.emit(TerminalEvent::Focused);
                }),
            )
            .relative()
            .size_full()
            .px(TERM_PAD_X)
            .py(TERM_PAD_Y)
            .font(mono_font())
            .text_size(TERM_FONT_SIZE)
            .child(grid)
            .children(message)
    }
}

#[cfg(test)]
mod tests {
    // gpui_kit の glob は test 属性マクロを含むため、必要な名前だけを取り込む。
    use super::{CellMetrics, GridSize, point_to_cell};
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

    #[test]
    fn point_to_cell_clamps() {
        assert_eq!(point_to_cell(point(px(-5.), px(-5.)), metrics(), SIZE), (0, 0, false));
        assert_eq!(point_to_cell(point(px(10000.), px(10000.)), metrics(), SIZE), (23, 79, true));
    }
}
