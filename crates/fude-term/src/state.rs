//! 端末の状態（alacritty_terminal の Term）と、描画用のスナップショット。

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, Processor, StdSyncHandler};
use fude_core::keys::TermModes;

use crate::palette::Palette;

/// 端末の升目の大きさ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

impl GridSize {
    /// 描画領域の大きさ（px）から升目の数を求める。切り捨て。最小 2 列 1 行。
    pub fn from_pixels(width: f32, height: f32, cell_w: f32, cell_h: f32) -> GridSize {
        let count = |len: f32, cell: f32| if cell > 0.0 { (len / cell).floor().max(0.0) as u16 } else { 0 };
        GridSize { cols: count(width, cell_w).max(2), rows: count(height, cell_h).max(1) }
    }

    fn term_size(self) -> TermSize {
        TermSize::new(usize::from(self.cols), usize::from(self.rows))
    }
}

/// 端末から UI や PTY へ知らせること。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermEvent {
    /// PTY に書き戻すバイト列（DSR の応答など）。
    Write(Vec<u8>),
    /// OSC の題名。None は既定に戻す。
    Title(Option<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectKind {
    Simple,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    Block,
    Beam,
    Underline,
    Hollow,
}

/// 描画用の 1 セル。色は解決済み。
#[derive(Clone, Debug, PartialEq)]
pub struct SnapCell {
    pub c: char,
    /// 結合文字など、幅を持たない文字。
    pub zerowidth: Vec<char>,
    pub fg: u32,
    /// None は既定の背景（塗らない）。
    pub bg: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub dim: bool,
    /// 全角の左半分。
    pub wide: bool,
    /// 全角の右半分、または行末の送り用の空き。描かない。
    pub spacer: bool,
    pub selected: bool,
    /// ターミナル内の検索の一致。
    pub matched: bool,
    /// 選んでいる一致。
    pub current_match: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapCursor {
    pub row: u16,
    pub col: u16,
    pub kind: CursorKind,
    /// 全角の上にあるか（2 セル分で描く）。
    pub wide: bool,
}

/// 描画用の画面の写し。lines[row][col]。
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<Vec<SnapCell>>,
    pub cursor: Option<SnapCursor>,
    pub display_offset: usize,
}

impl Snapshot {
    /// 行の文字列。spacer を除き、右端の空白を削る。
    pub fn row_text(&self, row: u16) -> String {
        let Some(line) = self.lines.get(usize::from(row)) else { return String::new() };
        let mut text = String::new();
        for cell in line.iter().filter(|c| !c.spacer) {
            text.push(cell.c);
            text.extend(&cell.zerowidth);
        }
        text.trim_end().to_string()
    }

    /// 行の文字と、その文字が始まる列（spacer を除く。全角は 2 列を使う）。リンクの位置をセルに戻すために使う。
    pub fn row_chars(&self, row: u16) -> Vec<(char, u16)> {
        let Some(line) = self.lines.get(usize::from(row)) else { return Vec::new() };
        line.iter().enumerate().filter(|(_, c)| !c.spacer).map(|(i, c)| (c.c, i as u16)).collect()
    }
}

/// Term からの通知を溜める。
#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let ev = match event {
            Event::PtyWrite(text) => TermEvent::Write(text.into_bytes()),
            Event::Title(title) => TermEvent::Title(Some(title)),
            Event::ResetTitle => TermEvent::Title(None),
            _ => return,
        };
        self.0.lock().expect("端末の通知のロック").push(ev);
    }
}

/// ターミナル内の検索で数える一致の上限。
pub const TERM_SEARCH_MAX: usize = 1000;

/// 出力のあとに一致を探し直す間隔の最小（出力の流れている間、履歴全体を毎回は探さない）。
pub const SEARCH_RESCAN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);

/// ターミナル内の検索の状態。一致は履歴を含む全体から集めて控える。
struct TermSearch {
    regex: RegexSearch,
    matches: Vec<Match>,
    current: Option<usize>,
    /// 出力が来て、一致の位置がずれているかもしれない。
    dirty: bool,
    last_scan: std::time::Instant,
}

/// 正規表現の記号を逃がし、文字どおりに探す形にする。
fn escape_literal(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// 履歴を含む全体の一致（上から順）。TERM_SEARCH_MAX を超えるときは、下（新しい、カーソルに近い）ほうを残す。
fn collect_matches(term: &Term<Listener>, regex: &mut RegexSearch) -> Vec<Match> {
    let top = Point::new(term.topmost_line(), Column(0));
    let bottom = Point::new(term.bottommost_line(), term.last_column());
    let mut matches: Vec<Match> = RegexIter::new(bottom, top, Direction::Left, term, regex).take(TERM_SEARCH_MAX).collect();
    matches.reverse();
    matches
}

/// 端末の状態。PTY には依存しない。
pub struct TermState {
    term: Term<Listener>,
    parser: Processor<StdSyncHandler>,
    events: Listener,
    size: GridSize,
    search: Option<TermSearch>,
    scrollback: usize,
}

impl TermState {
    pub fn new(size: GridSize, scrollback: usize) -> TermState {
        let events = Listener::default();
        let config = Config { scrolling_history: scrollback, ..Config::default() };
        let term = Term::new(config, &size.term_size(), events.clone());
        TermState { term, parser: Processor::new(), events, size, search: None, scrollback }
    }

    /// アプリが形を指定していないときのカーソルの形（設定の [cursor] shape）。アプリの DECSCUSR が優先する。
    pub fn set_default_cursor(&mut self, shape: fude_core::settings::CursorShape) {
        use alacritty_terminal::vte::ansi::CursorStyle;
        use fude_core::settings::CursorShape as Setting;
        let shape = match shape {
            Setting::Block => CursorShape::Block,
            Setting::Bar => CursorShape::Beam,
            Setting::Underline => CursorShape::Underline,
        };
        let config = Config {
            scrolling_history: self.scrollback,
            default_cursor_style: CursorStyle { shape, blinking: false },
            ..Config::default()
        };
        self.term.set_options(config);
    }

    /// PTY の出力を解析して画面に反映する。検索中なら、ずれた一致を探し直す。
    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        if let Some(s) = self.search.as_mut() {
            s.dirty = true;
            if s.last_scan.elapsed() >= SEARCH_RESCAN_INTERVAL {
                self.rescan_search();
            }
        }
    }

    /// query（文字どおり、スマートケース）で探し直し、カーソルに近い（下から上へ見て最初の）一致を選んで、
    /// 見える位置へスクロールする。戻り値は（選んだ一致の番号 1 始まり, 一致の数）。
    pub fn search(&mut self, query: &str) -> (usize, usize) {
        if query.is_empty() {
            self.clear_search();
            return (0, 0);
        }
        let Ok(mut regex) = RegexSearch::new(&escape_literal(query)) else {
            self.clear_search();
            return (0, 0);
        };
        let matches = collect_matches(&self.term, &mut regex);
        let cursor = self.term.grid().cursor.point;
        let current = matches.iter().rposition(|m| *m.start() <= cursor).or_else(|| matches.len().checked_sub(1));
        self.search = Some(TermSearch { regex, matches, current, dirty: false, last_scan: std::time::Instant::now() });
        self.reveal_current();
        self.search_position()
    }

    /// 次（forward = true は下へ）の一致を選んでスクロールする。端を越えたら反対の端へ回る。
    pub fn search_step(&mut self, forward: bool) -> (usize, usize) {
        if self.search.as_ref().is_some_and(|s| s.dirty) {
            self.rescan_search();
        }
        let Some(s) = self.search.as_mut() else { return (0, 0) };
        let n = s.matches.len();
        if n == 0 {
            return (0, 0);
        }
        s.current = Some(match (s.current, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        });
        self.reveal_current();
        self.search_position()
    }

    pub fn clear_search(&mut self) {
        self.search = None;
    }

    /// 描画の前に呼ぶ。検索中なら（今の位置, まだ探し直しを待っているか）を返す。
    /// 間隔が空いていれば探し直す。待っている間は true（呼び出し側はもう一度描く）。
    pub fn refresh_search(&mut self) -> Option<((usize, usize), bool)> {
        let s = self.search.as_ref()?;
        if s.dirty && s.last_scan.elapsed() >= SEARCH_RESCAN_INTERVAL {
            self.rescan_search();
        }
        let pending = self.search.as_ref().is_some_and(|s| s.dirty);
        Some((self.search_position(), pending))
    }

    /// 出力や大きさの変更のあと、同じ条件で探し直す。選んでいた番号はできるだけ保つ（スクロールはしない）。
    fn rescan_search(&mut self) {
        let Some(s) = self.search.as_mut() else { return };
        s.dirty = false;
        s.last_scan = std::time::Instant::now();
        s.matches = collect_matches(&self.term, &mut s.regex);
        s.current = match (s.current, s.matches.len()) {
            (_, 0) => None,
            (Some(i), n) => Some(i.min(n - 1)),
            (None, n) => Some(n - 1),
        };
    }

    pub fn search_position(&self) -> (usize, usize) {
        match &self.search {
            Some(TermSearch { matches, current: Some(i), .. }) => (i + 1, matches.len()),
            _ => (0, 0),
        }
    }

    fn reveal_current(&mut self) {
        let Some(point) = self.search.as_ref().and_then(|s| s.matches.get(s.current?)).map(|m| *m.start()) else { return };
        self.term.scroll_to_point(point);
    }

    pub fn take_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut *self.events.0.lock().expect("端末の通知のロック"))
    }

    pub fn resize(&mut self, size: GridSize) {
        if size != self.size {
            self.size = size;
            self.term.resize(size.term_size());
            self.rescan_search();
        }
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    pub fn snapshot(&self, palette: &Palette, focused: bool) -> Snapshot {
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        let (cols, rows) = (self.size.cols, self.size.rows);
        let mut lines = vec![Vec::with_capacity(usize::from(cols)); usize::from(rows)];
        // 見えている行に掛かる一致だけを残す（セルごとに全部の一致を調べない）。
        let (top, bottom) = (Line(-offset), Line(-offset + i32::from(rows) - 1));
        let visible: Vec<(&Match, bool)> = self
            .search
            .iter()
            .flat_map(|s| s.matches.iter().enumerate().map(move |(i, m)| (m, s.current == Some(i))))
            .filter(|(m, _)| m.end().line >= top && m.start().line <= bottom)
            .collect();
        for indexed in content.display_iter {
            let row = indexed.point.line.0 + offset;
            let Some(line) = usize::try_from(row).ok().and_then(|r| lines.get_mut(r)) else { continue };
            let cell = indexed.cell;
            let flags = cell.flags;
            let mut fg = palette.resolve_fg(cell.fg, content.colors);
            let mut bg = palette.resolve_bg(cell.bg, content.colors);
            if flags.contains(Flags::INVERSE) {
                (fg, bg) = (bg.unwrap_or(palette.background), Some(fg));
            }
            let hidden = flags.contains(Flags::HIDDEN);
            line.push(SnapCell {
                c: if hidden { ' ' } else { cell.c },
                zerowidth: if hidden { Vec::new() } else { cell.zerowidth().map(<[char]>::to_vec).unwrap_or_default() },
                fg,
                bg,
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES),
                strike: flags.contains(Flags::STRIKEOUT),
                dim: flags.contains(Flags::DIM),
                wide: flags.contains(Flags::WIDE_CHAR),
                spacer: flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
                selected: content.selection.is_some_and(|sel| sel.contains(indexed.point)),
                matched: visible.iter().any(|(m, _)| m.contains(&indexed.point)),
                current_match: visible.iter().any(|(m, cur)| *cur && m.contains(&indexed.point)),
            });
        }
        let cursor = (|| {
            let kind = match content.cursor.shape {
                CursorShape::Hidden => return None,
                _ if !focused => CursorKind::Hollow,
                CursorShape::Block => CursorKind::Block,
                CursorShape::Beam => CursorKind::Beam,
                CursorShape::Underline => CursorKind::Underline,
                CursorShape::HollowBlock => CursorKind::Hollow,
            };
            let point = content.cursor.point;
            let row = u16::try_from(point.line.0 + offset).ok().filter(|&r| r < rows)?;
            let col = u16::try_from(point.column.0).ok()?;
            let wide = lines[usize::from(row)].get(usize::from(col)).is_some_and(|c| c.wide);
            Some(SnapCursor { row, col, kind, wide })
        })();
        Snapshot { cols, rows, lines, cursor, display_offset: content.display_offset }
    }

    pub fn modes(&self) -> TermModes {
        TermModes { app_cursor: self.term.mode().contains(TermMode::APP_CURSOR) }
    }

    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// アプリが求めたマウスの報告。
    pub fn mouse_mode(&self) -> crate::input::MouseMode {
        use crate::input::{MouseMode, MouseReport};
        let mode = self.term.mode();
        let report = if mode.contains(TermMode::MOUSE_MOTION) {
            MouseReport::Motion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseReport::Drag
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseReport::Click
        } else {
            MouseReport::Off
        };
        MouseMode { report, sgr: mode.contains(TermMode::SGR_MOUSE) }
    }

    /// 代替画面で、ホイールを矢印キーとして送るか。
    pub fn alt_scroll(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
    }

    /// 表示をスクロールする。正で過去へ。
    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    fn grid_point(&self, row: u16, col: u16) -> Point {
        let col = col.min(self.size.cols.saturating_sub(1));
        Point::new(Line(i32::from(row) - self.display_offset() as i32), Column(usize::from(col)))
    }

    pub fn start_selection(&mut self, row: u16, col: u16, right_half: bool, kind: SelectKind) {
        let ty = match kind {
            SelectKind::Simple => SelectionType::Simple,
            SelectKind::Word => SelectionType::Semantic,
            SelectKind::Line => SelectionType::Lines,
        };
        let side = if right_half { Side::Right } else { Side::Left };
        self.term.selection = Some(Selection::new(ty, self.grid_point(row, col), side));
    }

    pub fn update_selection(&mut self, row: u16, col: u16, right_half: bool) {
        let point = self.grid_point(row, col);
        if let Some(sel) = self.term.selection.as_mut() {
            sel.update(point, if right_half { Side::Right } else { Side::Left });
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::tests::test_palette;

    fn term(cols: u16, rows: u16, input: &str) -> TermState {
        let mut t = TermState::new(GridSize { cols, rows }, 100);
        t.advance(input.as_bytes());
        t
    }

    fn snap(t: &TermState) -> Snapshot {
        t.snapshot(&test_palette(), true)
    }

    /// Claude Code の全画面の表示（tui: fullscreen）が送る組み合わせ。
    #[test]
    fn mouse_mode_follows_decset() {
        use crate::input::{MouseMode, MouseReport};
        let t = term(20, 5, "");
        assert_eq!(t.mouse_mode(), MouseMode { report: MouseReport::Off, sgr: false });
        let t = term(20, 5, "\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
        assert_eq!(t.mouse_mode(), MouseMode { report: MouseReport::Motion, sgr: true });
        let t = term(20, 5, "\x1b[?1002h\x1b[?1006h\x1b[?1006l");
        assert_eq!(t.mouse_mode(), MouseMode { report: MouseReport::Drag, sgr: false });
        let t = term(20, 5, "\x1b[?1000h\x1b[?1000l");
        assert!(!t.mouse_mode().is_on());
    }

    #[test]
    fn search_finds_and_steps_with_wrap() {
        let mut t = TermState::new(GridSize { cols: 20, rows: 5 }, 100);
        t.advance(b"foo 1\r\nbar\r\nFoo 2\r\nfoo 3\r\n");
        // スマートケース: 小文字だけなら大文字も一致する。
        assert_eq!(t.search("foo"), (3, 3));
        assert_eq!(t.search_step(false), (2, 3));
        assert_eq!(t.search_step(true), (3, 3));
        assert_eq!(t.search_step(true), (1, 3));
        assert_eq!(t.search("Foo"), (1, 1));
        assert_eq!(t.search(""), (0, 0));
    }

    #[test]
    fn search_marks_cells_in_snapshot() {
        let mut t = TermState::new(GridSize { cols: 20, rows: 5 }, 100);
        t.advance(b"a.b axb\r\n");
        t.search("a.b"); // "." は文字どおり
        let s = snap(&t);
        let marked: Vec<bool> = s.lines[0][..7].iter().map(|c| c.matched).collect();
        assert_eq!(marked, [true, true, true, false, false, false, false]);
        assert!(s.lines[0][0].current_match);
        t.clear_search();
        assert!(!snap(&t).lines[0][0].matched);
    }

    #[test]
    fn search_scrolls_to_history() {
        let mut t = TermState::new(GridSize { cols: 20, rows: 3 }, 100);
        t.advance(b"needle\r\n");
        for _ in 0..10 {
            t.advance(b"x\r\n");
        }
        assert_eq!(t.search("needle"), (1, 1));
        assert!(t.display_offset() > 0);
    }

    #[test]
    fn search_keeps_newest_matches_near_cursor() {
        // 上限を超えたら、カーソルに近い（新しい）ほうを残す。選んだ一致は画面の中にある。
        let mut t = TermState::new(GridSize { cols: 20, rows: 5 }, 3000);
        for i in 0..1200 {
            t.advance(format!("hit {i}\r\n").as_bytes());
        }
        assert_eq!(t.search("hit"), (TERM_SEARCH_MAX, TERM_SEARCH_MAX));
        assert_eq!(t.display_offset(), 0);
        assert!(snap(&t).lines.iter().any(|l| l.iter().any(|c| c.current_match)));
    }

    #[test]
    fn output_rescan_is_throttled() {
        // 出力のたびに履歴全体を探し直さない（読む側のロックを長く持たない）。描画の前に refresh_search で追いつく。
        let mut t = TermState::new(GridSize { cols: 20, rows: 5 }, 100);
        t.advance(b"foo\r\n");
        assert_eq!(t.search("foo"), (1, 1));
        t.advance(b"foo\r\n");
        assert_eq!(t.search_position(), (1, 1));
        std::thread::sleep(SEARCH_RESCAN_INTERVAL);
        let (pos, pending) = t.refresh_search().unwrap();
        assert_eq!((pos.1, pending), (2, false));
        t.clear_search();
        assert!(t.refresh_search().is_none());
    }

    #[test]
    fn search_follows_new_output() {
        // 出力が来ても、控えた一致の位置が新しい中身に合う（探し直す）。
        let mut t = TermState::new(GridSize { cols: 20, rows: 3 }, 100);
        t.advance(b"foo\r\n");
        assert_eq!(t.search("foo"), (1, 1));
        t.advance(b"x\r\ny\r\nfoo\r\n");
        assert_eq!(t.search_step(true).1, 2);
    }

    #[test]
    fn row_chars_map_wide_chars_to_columns() {
        let s = snap(&term(20, 5, "あa b"));
        let chars = s.row_chars(0);
        assert_eq!(&chars[..4], &[('あ', 0), ('a', 2), (' ', 3), ('b', 4)]);
        assert!(s.row_chars(99).is_empty());
    }

    #[test]
    fn plain_text_and_cursor() {
        let s = snap(&term(20, 5, "hi"));
        assert_eq!(s.row_text(0), "hi");
        let c = s.cursor.unwrap();
        assert_eq!((c.row, c.col, c.kind), (0, 2, CursorKind::Block));
    }

    #[test]
    fn sgr_colors_resolve() {
        let s = snap(&term(20, 5, "\x1b[31mR\x1b[0m"));
        assert_eq!(s.lines[0][0].fg, test_palette().ansi[1]);
        assert_eq!(s.lines[0][0].bg, None);
    }

    #[test]
    fn inverse_swaps_colors() {
        let s = snap(&term(20, 5, "\x1b[7mX"));
        let p = test_palette();
        assert_eq!(s.lines[0][0].bg, Some(p.foreground));
        assert_eq!(s.lines[0][0].fg, p.background);
    }

    #[test]
    fn wide_char_occupies_two_cells() {
        let s = snap(&term(20, 5, "あa"));
        assert!(s.lines[0][0].wide);
        assert!(s.lines[0][1].spacer);
        assert_eq!(s.lines[0][2].c, 'a');
    }

    #[test]
    fn wide_char_at_line_end_wraps() {
        let s = snap(&term(5, 3, "abcdあ"));
        assert!(s.lines[0][4].spacer);
        assert_eq!(s.row_text(0), "abcd");
        assert_eq!(s.row_text(1), "あ");
    }

    #[test]
    fn combining_char_stays_with_base() {
        let s = snap(&term(20, 5, "e\u{301}"));
        assert_eq!(s.lines[0][0].c, 'e');
        assert_eq!(s.lines[0][0].zerowidth, vec!['\u{301}']);
    }

    #[test]
    fn dsr_reply_is_write_event() {
        let mut t = term(20, 5, "\x1b[6n");
        assert_eq!(t.take_events(), vec![TermEvent::Write(b"\x1b[1;1R".to_vec())]);
    }

    #[test]
    fn osc_title_events() {
        let mut t = term(20, 5, "\x1b]0;hi\x07");
        assert_eq!(t.take_events(), vec![TermEvent::Title(Some("hi".into()))]);
    }

    #[test]
    fn unfocused_cursor_is_hollow() {
        let t = term(20, 5, "a");
        assert_eq!(t.snapshot(&test_palette(), false).cursor.unwrap().kind, CursorKind::Hollow);
    }

    #[test]
    fn hidden_cursor_is_none() {
        assert!(snap(&term(20, 5, "\x1b[?25l")).cursor.is_none());
    }

    #[test]
    fn scroll_moves_display_offset() {
        let lines: String = (0..10).map(|i| format!("{i}\r\n")).collect();
        let mut t = term(10, 3, &lines);
        t.scroll(2);
        assert_eq!(t.display_offset(), 2);
        assert!(snap(&t).cursor.is_none());
        t.scroll_to_bottom();
        assert_eq!(t.display_offset(), 0);
    }

    #[test]
    fn selection_text_simple() {
        let mut t = term(20, 5, "hello world");
        t.start_selection(0, 0, false, SelectKind::Simple);
        t.update_selection(0, 4, true);
        assert_eq!(t.selection_text().as_deref(), Some("hello"));
        assert!(snap(&t).lines[0][0].selected);
        t.clear_selection();
        assert_eq!(t.selection_text(), None);
    }

    #[test]
    fn app_cursor_and_bracketed_modes() {
        let t = term(20, 5, "\x1b[?1h\x1b[?2004h");
        assert!(t.modes().app_cursor);
        assert!(t.bracketed_paste());
    }

    #[test]
    fn grid_size_from_pixels_clamps_minimum() {
        assert_eq!(GridSize::from_pixels(0., 0., 8., 18.), GridSize { cols: 2, rows: 1 });
        assert_eq!(GridSize::from_pixels(800., 360., 8., 18.), GridSize { cols: 100, rows: 20 });
    }

    #[test]
    fn default_cursor_shape_applies_until_app_sets_one() {
        use crate::state::CursorKind;
        use fude_core::settings::CursorShape as Setting;
        let mut t = TermState::new(GridSize { cols: 20, rows: 5 }, 100);
        assert_eq!(snap(&t).cursor.map(|c| c.kind), Some(CursorKind::Block));
        t.set_default_cursor(Setting::Bar);
        assert_eq!(snap(&t).cursor.map(|c| c.kind), Some(CursorKind::Beam));
        t.set_default_cursor(Setting::Underline);
        assert_eq!(snap(&t).cursor.map(|c| c.kind), Some(CursorKind::Underline));
        // アプリが DECSCUSR で指定したら、そちらが勝つ。
        t.advance(b"\x1b[2 q");
        assert_eq!(snap(&t).cursor.map(|c| c.kind), Some(CursorKind::Block));
        // 既定に戻す指定（0）では設定の形に戻る。
        t.advance(b"\x1b[0 q");
        assert_eq!(snap(&t).cursor.map(|c| c.kind), Some(CursorKind::Underline));
    }
}
