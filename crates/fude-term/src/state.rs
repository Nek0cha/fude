//! 端末の状態（alacritty_terminal の Term）と、描画用のスナップショット。

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
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

/// 端末の状態。PTY には依存しない。
pub struct TermState {
    term: Term<Listener>,
    parser: Processor<StdSyncHandler>,
    events: Listener,
    size: GridSize,
}

impl TermState {
    pub fn new(size: GridSize, scrollback: usize) -> TermState {
        let events = Listener::default();
        let config = Config { scrolling_history: scrollback, ..Config::default() };
        let term = Term::new(config, &size.term_size(), events.clone());
        TermState { term, parser: Processor::new(), events, size }
    }

    /// PTY の出力を解析して画面に反映する。
    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn take_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut *self.events.0.lock().expect("端末の通知のロック"))
    }

    pub fn resize(&mut self, size: GridSize) {
        if size != self.size {
            self.size = size;
            self.term.resize(size.term_size());
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
}
