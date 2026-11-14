//! ペインごとのターミナルのビュー。セッション、フォーカス、入力、IME を受け持つ。

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use fude_core::keymap::Platform;
use fude_core::keys::{KeyOverrides, KeyRoute, RawKey, encode_key, route_key};
use fude_core::workspace::{ProjectRoot, ShellKind};
use fude_term::{
    GridSize, MouseAction, MouseKey, MouseMods, SelectKind, Session, SessionEvent, WheelAccum, alt_scroll_bytes, alt_scroll_lines,
    mouse_bytes, paste_bytes, spawn_spec,
};
use fude_term::links::{LinkTarget, find_links};
use futures::StreamExt;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::hover::{HoverFades, HoverKey};
use crate::term_search::{TermSearchBar, TermSearchClose, bar_button};
use crate::terminal_paint::{CellMetrics, paint_terminal};
use crate::theme::size::{TERM_FONT_SIZE, TERM_PAD_X, TERM_PAD_Y};
use crate::theme::{Theme, mono_font};
use crate::title_row::icon;
use crate::workspace_view::{CopySelection, PasteText};
use fude_core::motion::Fade;
use gpui_kit::base::input::{InputEvent, InputState};

/// 「最新へ」のボタンの大きさと、ペインの端からの距離。
const TO_BOTTOM_SIZE: Pixels = px(28.);
const TO_BOTTOM_INSET: Pixels = px(12.);

/// ビューからワークスペースへ知らせること。
pub enum TerminalEvent {
    /// 出力の中のパスや URL をクリックした。
    OpenLink(LinkTarget),
    Title(Option<String>),
    Exited { success: bool },
    /// クリックでこのペインにフォーカスした。
    Focused,
    /// キー入力、IME の確定、貼り付けで PTY に書いた（返事待ちを消す）。
    Input,
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
    /// アプリへ押したと報告したボタン（離したときに報告する）。
    mouse_down: Option<MouseKey>,
    /// 最後に移動を報告したセル（同じセルの中の移動は送らない）。
    mouse_cell: Option<(u16, u16)>,
    /// マウスが乗っているリンク（行、始まりの列、終わりの列（含まない）、行き先）。
    hover_link: Option<(u16, u16, u16, LinkTarget)>,
    /// 動きの間は PTY にサイズを送らない（描画の closure から読むため Cell）。
    resize_frozen: Rc<Cell<bool>>,
    /// 作った時刻（中身のフェードイン）。
    created_at: Instant,
    /// ターミナル内の検索の欄。
    search: Option<TermSearchBar>,
    _events: Task<()>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl TerminalView {
    /// terminal_id は FUDE_TERMINAL_ID として渡す（`p<ペインの番号>`）。
    pub fn new(kind: &ShellKind, root: &ProjectRoot, terminal_id: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<SessionEvent>();
        let spec = spawn_spec(kind, root, terminal_id, &|k| std::env::var(k).ok());
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
            mouse_down: None,
            mouse_cell: None,
            hover_link: None,
            resize_frozen: Rc::new(Cell::new(false)),
            created_at: Instant::now(),
            search: None,
            _events: events,
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// 動きの間は true にする。false に戻したら、次の描画で最終的なサイズを 1 回だけ送る。
    pub fn set_resize_frozen(&mut self, frozen: bool, cx: &mut Context<Self>) {
        if self.resize_frozen.get() != frozen {
            self.resize_frozen.set(frozen);
            if !frozen {
                cx.notify();
            }
        }
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

    /// 検索の欄を出して入力欄へフォーカスする。出ていれば入力欄へフォーカスするだけ。
    pub(crate) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        if let Some(bar) = self.search.as_mut().filter(|b| !b.closing) {
            bar.input.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("ターミナル内を検索"));
        let sub = cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, _, cx| match ev {
            InputEvent::Change => {
                let query = input.read(cx).value().to_string();
                let counts = this.session.as_ref().map_or((0, 0), |s| s.with_state(|st| st.search(&query)));
                if let Some(bar) = this.search.as_mut() {
                    bar.counts = counts;
                }
                cx.notify();
            }
            // Enter は上へ（古いほう）、Shift+Enter は下へ。
            InputEvent::PressEnter { shift, .. } => this.step_search(*shift, cx),
            _ => {}
        });
        let mut fade = Fade::settled(0.0);
        fade.animate_to(1.0, crate::theme::motion::POPUP_IN, crate::theme::motion::ease_out_quint, now);
        input.update(cx, |s, cx| s.focus(window, cx));
        self.search = Some(TermSearchBar { input, counts: (0, 0), fade, closing: false, _sub: sub });
        cx.notify();
    }

    /// 検索の欄を閉じ、一致の塗りを消して、ターミナルへフォーカスを戻す。
    pub(crate) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bar) = self.search.as_mut().filter(|b| !b.closing) else { return };
        bar.closing = true;
        bar.fade.animate_to(0.0, crate::theme::motion::POPUP_OUT, crate::theme::motion::ease_out_quint, Instant::now());
        if let Some(s) = &self.session {
            s.with_state(|st| st.clear_search());
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn step_search(&mut self, forward: bool, cx: &mut Context<Self>) {
        let counts = self.session.as_ref().map_or((0, 0), |s| s.with_state(|st| st.search_step(forward)));
        if let Some(bar) = self.search.as_mut() {
            bar.counts = counts;
        }
        cx.notify();
    }

    fn render_search(&mut self, theme: &Theme, now: Instant, window: &mut Window, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let bar = self.search.as_ref()?;
        if bar.closing && !bar.fade.is_animating(now) {
            self.search = None;
            return None;
        }
        if bar.fade.is_animating(now) {
            window.request_animation_frame();
        }
        let query = bar.input.read(cx).value().to_string();
        // 出力で一致がずれたら、間隔を空けて探し直し、数も合わせる。
        if !bar.closing
            && let Some((counts, pending)) = self.session.as_ref().and_then(|s| s.with_state(|st| st.refresh_search()))
        {
            if pending {
                window.request_animation_frame();
            }
            if let Some(bar) = self.search.as_mut() {
                bar.counts = counts;
            }
        }
        let bar = self.search.as_ref()?;
        Some(
            bar.frame(&query, theme, now)
                .on_action(cx.listener(|this, _: &TermSearchClose, window, cx| this.close_search(window, cx)))
                .child(bar_button("term-search-up", IconName::ArrowUp, theme).on_click(cx.listener(|this, _, _, cx| this.step_search(false, cx))))
                .child(bar_button("term-search-down", IconName::ArrowDown, theme).on_click(cx.listener(|this, _, _, cx| this.step_search(true, cx))))
                .child(bar_button("term-search-close", IconName::X, theme).on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx)))),
        )
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
                cx.emit(TerminalEvent::Input);
            }
            cx.stop_propagation();
        }
    }

    /// アプリ（Claude Code の全画面の表示、vim など）がマウスを求めていて、Shift を押していなければ報告する。
    /// 報告の対象なら true（ターミナル側の選択やスクロールはしない）。Shift を押せば、いつもどおり選択できる。
    fn report_mouse(&mut self, key: MouseKey, action: MouseAction, pos: Point<Pixels>, modifiers: &Modifiers) -> bool {
        let Some(session) = &self.session else { return false };
        let mode = session.with_state(|st| st.mouse_mode());
        if !mode.is_on() || modifiers.shift {
            return false;
        }
        let Some((row, col, _)) = self.cell_at(pos) else { return false };
        if action == MouseAction::Move && self.mouse_cell == Some((row, col)) {
            return true;
        }
        self.mouse_cell = Some((row, col));
        let mods = MouseMods { shift: false, alt: modifiers.alt, ctrl: modifiers.control };
        if let Some(bytes) = mouse_bytes(mode, key, action, row, col, mods) {
            session.write(bytes);
        }
        true
    }

    /// 報告したボタンを離した（ビューの外で離したときも）。
    fn release_mouse(&mut self, button: MouseKey, pos: Point<Pixels>, modifiers: &Modifiers) -> bool {
        if self.mouse_down != Some(button) {
            return false;
        }
        self.mouse_down = None;
        self.mouse_cell = None;
        // Shift を押して離しても、押したことは報告してあるので離したことも送る。
        let plain = Modifiers { shift: false, ..*modifiers };
        self.report_mouse(button, MouseAction::Release, pos, &plain);
        true
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.emit(TerminalEvent::Focused);
        self.mouse_cell = None;
        if self.report_mouse(MouseKey::Left, MouseAction::Press, ev.position, &ev.modifiers) {
            self.mouse_down = Some(MouseKey::Left);
            if let Some(s) = &self.session {
                s.with_state(|st| st.clear_selection());
            }
            cx.notify();
            return;
        }
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

    /// マウスが乗った行だけリンクを探す（常に下線を出すと画面が散らかるため）。
    fn update_hover_link(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let found = (|| {
            let session = self.session.as_ref()?;
            let (row, col, _) = self.cell_at(pos)?;
            let palette = cx.global::<Theme>().term_palette;
            let chars = session.with_state(|st| st.snapshot(&palette, false).row_chars(row));
            let (start, end, target) = link_at(&chars, col)?;
            Some((row, start, end, target))
        })();
        if found != self.hover_link {
            self.hover_link = found;
            cx.notify();
        }
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(key) = self.mouse_down {
            self.report_mouse(key, MouseAction::Move, ev.position, &ev.modifiers);
            return;
        }
        if ev.pressed_button.is_none() && self.report_mouse(MouseKey::None, MouseAction::Move, ev.position, &ev.modifiers) {
            if self.hover_link.take().is_some() {
                cx.notify();
            }
            return;
        }
        if ev.pressed_button.is_none() {
            self.update_hover_link(ev.position, cx);
            return;
        }
        if self.drag.is_none() || ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (Some(session), Some((row, col, right))) = (&self.session, self.cell_at(ev.position)) else { return };
        session.with_state(|st| st.update_selection(row, col, right));
        self.drag = Some(true);
        cx.notify();
    }

    fn on_right_click(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.mouse_cell = None;
        if self.report_mouse(MouseKey::Right, MouseAction::Press, ev.position, &ev.modifiers) {
            self.mouse_down = Some(MouseKey::Right);
            return;
        }
        let Some(session) = self.session.clone() else { return };
        let has_selection = session.with_state(|st| st.selection_text().is_some());
        match right_click_action(has_selection) {
            RightClick::Copy => {
                self.copy(cx);
                session.with_state(|st| st.clear_selection());
            }
            RightClick::Paste => self.paste(cx),
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.release_mouse(MouseKey::Left, ev.position, &ev.modifiers) {
            return;
        }
        // 押して動かさずに離したら、乗っているリンクを開く（ドラッグなら今までどおり選択）。
        if self.drag == Some(false)
            && self.click_count == 1
            && let Some((_, _, _, target)) = self.hover_link.clone()
        {
            self.drag = None;
            if let Some(s) = &self.session {
                s.with_state(|st| st.clear_selection());
            }
            cx.emit(TerminalEvent::OpenLink(target));
            cx.notify();
            return;
        }
        if let Some(moved) = self.drag.take()
            && !made_selection(moved, self.click_count)
            && let Some(s) = &self.session
        {
            // 押しただけで動かさなかったクリックは、選択を残さない。
            s.with_state(|st| st.clear_selection());
        }
        cx.notify();
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return };
        let lines = match ev.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => p.y / self.metrics.line_h,
        };
        let report = session.with_state(|st| st.mouse_mode()).is_on() && !ev.modifiers.shift;
        if report {
            // Windows ではホイール 1 段を 1 回の報告にする（代替スクロールと同じ）。
            let n = self.wheel.feed(alt_scroll_lines(lines, cfg!(windows)));
            let key = if n > 0 { MouseKey::WheelUp } else { MouseKey::WheelDown };
            for _ in 0..n.unsigned_abs() {
                self.report_mouse(key, MouseAction::Press, ev.position, &ev.modifiers);
            }
            return;
        }
        let alt = session.with_state(|st| st.alt_scroll().then(|| st.modes().app_cursor));
        let lines = if alt.is_some() { alt_scroll_lines(lines, cfg!(windows)) } else { lines };
        let n = self.wheel.feed(lines);
        if n == 0 {
            return;
        }
        match alt {
            Some(app_cursor) => session.write(alt_scroll_bytes(n, app_cursor)),
            None => session.with_state(|st| st.scroll(n)),
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
        cx.emit(TerminalEvent::Input);
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

/// 描画で測った大きさ measured を PTY に送るか。動きの間（frozen）と、今と同じなら送らない（設計書 §6.3）。
pub(crate) fn resize_target(frozen: bool, measured: GridSize, current: GridSize) -> Option<GridSize> {
    (!frozen && measured != current).then_some(measured)
}

/// 行の文字と列の対応（Snapshot::row_chars）から、col の上にあるリンクと、その列の範囲を返す。
pub(crate) fn link_at(chars: &[(char, u16)], col: u16) -> Option<(u16, u16, LinkTarget)> {
    let text: String = chars.iter().map(|(c, _)| *c).collect();
    find_links(&text).into_iter().find_map(|link| {
        let start = chars.get(link.start)?.1;
        let end = match chars.get(link.end) {
            Some((_, c)) => *c,
            None => chars.get(link.end.checked_sub(1)?)?.1 + 1,
        };
        (start <= col && col < end).then_some((start, end, link.target))
    })
}

/// 右クリックで行うこと。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RightClick {
    Copy,
    Paste,
}

/// 右クリックは、選択があればコピー、無ければ貼り付け（Windows Terminal と同じ。ユーザーの決定）。
pub(crate) fn right_click_action(has_selection: bool) -> RightClick {
    if has_selection { RightClick::Copy } else { RightClick::Paste }
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
            cx.emit(TerminalEvent::Input);
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
        let search = self.render_search(&theme, now, window, cx);
        let focused = self.focus.is_focused(window);
        let entity = cx.entity();
        let focus = self.focus.clone();
        let session = self.session.clone();
        let marked = self.marked.clone();
        let metrics = self.metrics;
        let underline = self.hover_link.as_ref().map(|(row, start, end, _)| (*row, *start, *end));
        let link_color = theme.fg_default;
        let bounds_cell = self.bounds.clone();
        let frozen = self.resize_frozen.clone();
        let age = now.saturating_duration_since(self.created_at);
        let opacity = crate::theme::motion::ease_out((age.as_secs_f32() / crate::theme::motion::TERM_IN.as_secs_f32()).min(1.0));
        if opacity < 1.0 {
            window.request_animation_frame();
        }

        let grid = canvas(
            move |bounds, _, _| {
                bounds_cell.set(Some(bounds));
                if let Some(s) = &session {
                    let w = f32::from(bounds.size.width);
                    let h = f32::from(bounds.size.height);
                    let measured = GridSize::from_pixels(w, h, f32::from(metrics.cell_w), f32::from(metrics.line_h));
                    if let Some(size) = resize_target(frozen.get(), measured, s.with_state(|st| st.size())) {
                        s.resize(size);
                    }
                }
                session
            },
            move |bounds, session, window, cx| {
                if let Some(s) = session {
                    let snap = s.with_state(|st| st.snapshot(&theme.term_palette, focused));
                    paint_terminal(&snap, marked.as_deref(), bounds.origin, metrics, &theme, window, cx);
                    if let Some((row, start, end)) = underline {
                        let x = bounds.origin.x + metrics.cell_w * f32::from(start);
                        let y = bounds.origin.y + metrics.line_h * f32::from(row + 1) - px(2.);
                        let w = metrics.cell_w * f32::from(end - start);
                        window.paint_quad(fill(Bounds::new(point(x, y), size(w, px(1.))), link_color));
                    }
                }
                window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
            },
        )
        .size_full();

        let msg_color = if self.session.is_some() { theme.fg_dim } else { theme.danger };
        let message = self.message.clone().map(|msg| {
            div().absolute().left(TERM_PAD_X).bottom(TERM_PAD_Y).text_color(msg_color).child(msg)
        });
        let terminal = div()
            .id("terminal")
            .track_focus(&self.focus)
            .key_context("Terminal")
            // ターミナルへのドロップ（パスの入力）は別の段階で入れる。ウィンドウの「開く」には回さない。
            .on_drop(|_: &ExternalPaths, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key(ev, cx)))
            .on_action(cx.listener(|this, _: &CopySelection, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &PasteText, _, cx| this.paste(cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| this.on_mouse_down(ev, window, cx)),
            )
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.on_right_click(ev, window, cx)))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.on_mouse_move(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, _, cx| this.on_mouse_up(ev, cx)))
            .on_mouse_up(MouseButton::Right, cx.listener(|this, ev: &MouseUpEvent, _, _| {
                this.release_mouse(MouseKey::Right, ev.position, &ev.modifiers);
            }))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, _, _| {
                this.release_mouse(MouseKey::Left, ev.position, &ev.modifiers);
            }))
            .on_mouse_up_out(MouseButton::Right, cx.listener(|this, ev: &MouseUpEvent, _, _| {
                this.release_mouse(MouseKey::Right, ev.position, &ev.modifiers);
            }))
            .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| this.on_scroll(ev, cx)))
            .when(self.hover_link.is_some(), |d| d.cursor_pointer())
            .relative()
            .size_full()
            .opacity(opacity)
            .px(TERM_PAD_X)
            .py(TERM_PAD_Y)
            .font(mono_font())
            .text_size(TERM_FONT_SIZE)
            .child(grid)
            .children(message)
            .children(to_bottom);
        // 検索の欄はターミナルの外側に重ねる（欄の入力のキーがターミナルの on_key_down に流れないように）。
        div().relative().size_full().child(terminal).children(search)
    }
}

#[cfg(test)]
mod tests {
    // gpui_kit の glob は test 属性マクロを含むため、必要な名前だけを取り込む。
    use super::{CellMetrics, GridSize, RightClick, made_selection, point_to_cell, right_click_action};
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

    /// 右クリックは、選択があればコピー、無ければ貼り付け（Windows Terminal と同じ）。
    #[test]
    fn right_click_copies_selection_else_pastes() {
        assert_eq!(right_click_action(true), RightClick::Copy);
        assert_eq!(right_click_action(false), RightClick::Paste);
    }

    /// ドラッグか、ダブル・トリプルクリックで選んだときだけ、離したあとも選択を残す。
    #[test]
    fn selection_kept_only_after_selecting() {
        assert!(!made_selection(false, 1));
        assert!(made_selection(true, 1));
        assert!(made_selection(false, 2));
        assert!(made_selection(false, 3));
    }

    #[test]
    fn frozen_terminal_does_not_resize() {
        use super::resize_target;
        let a = GridSize { cols: 80, rows: 24 };
        let b = GridSize { cols: 100, rows: 24 };
        assert_eq!(resize_target(true, b, a), None);
        assert_eq!(resize_target(false, a, a), None);
        assert_eq!(resize_target(false, b, a), Some(b));
    }

    #[test]
    fn link_at_maps_columns() {
        use super::link_at;
        use fude_term::links::LinkTarget;
        // "あ src/a.rs" — 全角の「あ」は 2 列を使う。
        let chars = vec![('あ', 0), (' ', 2), ('s', 3), ('r', 4), ('c', 5), ('/', 6), ('a', 7), ('.', 8), ('r', 9), ('s', 10)];
        let (start, end, target) = link_at(&chars, 5).unwrap();
        assert_eq!((start, end), (3, 11));
        assert_eq!(target, LinkTarget::Path { path: "src/a.rs".into(), line: None, col: None });
        assert!(link_at(&chars, 1).is_none());
        assert!(link_at(&chars, 11).is_none());
    }

    #[test]
    fn point_to_cell_clamps() {
        assert_eq!(point_to_cell(point(px(-5.), px(-5.)), metrics(), SIZE), (0, 0, false));
        assert_eq!(point_to_cell(point(px(10000.), px(10000.)), metrics(), SIZE), (23, 79, true));
    }
}
