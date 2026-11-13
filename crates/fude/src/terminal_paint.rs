//! Snapshot を升目に描く。

use fude_term::{CursorKind, SnapCell, Snapshot};
use gpui_kit::*;

use crate::theme::size::{TERM_FONT_SIZE, TERM_LINE_HEIGHT};
use crate::theme::{Theme, mono_font};

/// 1 セルの大きさ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub cell_w: Pixels,
    pub line_h: Pixels,
}

impl CellMetrics {
    /// 等幅フォントの 'm' の送り幅を 1 セルの幅とする。
    pub fn measure(window: &Window) -> CellMetrics {
        let text = window.text_system();
        let font_id = text.resolve_font(&mono_font());
        let cell_w = text.advance(font_id, TERM_FONT_SIZE, 'm').map_or(px(8.), |s| s.width);
        CellMetrics { cell_w, line_h: TERM_LINE_HEIGHT }
    }
}

/// 0xRRGGBB と不透明度から色を作る。
fn color(rgb: u32, alpha: f32) -> Hsla {
    Rgba {
        r: ((rgb >> 16) & 0xff) as f32 / 255.,
        g: ((rgb >> 8) & 0xff) as f32 / 255.,
        b: (rgb & 0xff) as f32 / 255.,
        a: alpha,
    }
    .into()
}

fn cell_font(cell: &SnapCell) -> Font {
    let mut f = mono_font();
    if cell.bold {
        f.weight = FontWeight::BOLD;
    }
    if cell.italic {
        f.style = FontStyle::Italic;
    }
    f
}

fn text_run(cell: &SnapCell, len: usize, fg: Hsla) -> TextRun {
    TextRun {
        len,
        font: cell_font(cell),
        color: fg,
        background_color: None,
        underline: cell.underline.then(|| UnderlineStyle { color: Some(fg), thickness: px(1.), wavy: false }),
        strikethrough: cell.strike.then(|| StrikethroughStyle { color: Some(fg), thickness: px(1.) }),
    }
}

fn fg_of(cell: &SnapCell) -> Hsla {
    color(cell.fg, if cell.dim { 0.6 } else { 1.0 })
}

/// まとめて 1 回で描ける文字か（ASCII で、結合文字を持たない）。
fn is_simple(cell: &SnapCell) -> bool {
    cell.c.is_ascii() && cell.zerowidth.is_empty() && !cell.spacer
}

fn same_look(a: &SnapCell, b: &SnapCell) -> bool {
    (a.fg, a.bold, a.italic, a.underline, a.strike, a.dim) == (b.fg, b.bold, b.italic, b.underline, b.strike, b.dim)
}

/// 同じ値が続く範囲ごとに、横長の四角を塗る。
fn paint_spans(
    line: &[SnapCell],
    key: impl Fn(&SnapCell) -> Option<Hsla>,
    y: Pixels,
    origin_x: Pixels,
    m: CellMetrics,
    window: &mut Window,
) {
    let mut col = 0;
    while col < line.len() {
        let start = col;
        let value = key(&line[col]);
        while col < line.len() && key(&line[col]) == value {
            col += 1;
        }
        if let Some(c) = value {
            let bounds = Bounds::new(
                point(origin_x + m.cell_w * start as f32, y),
                size(m.cell_w * (col - start) as f32, m.line_h),
            );
            window.paint_quad(fill(bounds, c));
        }
    }
}

/// 1 文字（結合文字を含む）を、width セル分の幅の中央に描く。
#[allow(clippy::too_many_arguments)]
fn paint_glyph(text: String, run: TextRun, x: Pixels, y: Pixels, width: Pixels, line_h: Pixels, window: &mut Window, cx: &mut App) {
    let len = text.len();
    let shaped = window.text_system().shape_line(text.into(), TERM_FONT_SIZE, &[TextRun { len, ..run }], None);
    let dx = ((width - shaped.width) / 2.).max(px(0.));
    let _ = shaped.paint(point(x + dx, y), line_h, TextAlign::Left, None, window, cx);
}

/// スナップショットを origin（升目の左上）から描く。marked は IME の変換中の文字列。
pub fn paint_terminal(
    snap: &Snapshot,
    marked: Option<&str>,
    origin: Point<Pixels>,
    m: CellMetrics,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) {
    for (row, line) in snap.lines.iter().enumerate() {
        let y = origin.y + m.line_h * row as f32;
        paint_spans(line, |c| c.bg.map(|bg| color(bg, 1.0)), y, origin.x, m, window);
        let selection: Hsla = theme.selection.into();
        paint_spans(line, |c| c.selected.then_some(selection), y, origin.x, m, window);

        let mut col = 0;
        while col < line.len() {
            let cell = &line[col];
            let x = origin.x + m.cell_w * col as f32;
            if cell.spacer {
                col += 1;
            } else if is_simple(cell) {
                let start = col;
                let mut text = String::new();
                while col < line.len() && is_simple(&line[col]) && same_look(&line[col], cell) {
                    text.push(line[col].c);
                    col += 1;
                }
                if text.trim().is_empty() && !cell.underline && !cell.strike {
                    continue;
                }
                let run = text_run(cell, text.len(), fg_of(cell));
                // 送り幅をセルの幅に固定して、升目からずれないようにする。
                let shaped = window.text_system().shape_line(text.into(), TERM_FONT_SIZE, &[run], Some(m.cell_w));
                let _ = shaped.paint(point(origin.x + m.cell_w * start as f32, y), m.line_h, TextAlign::Left, None, window, cx);
            } else {
                let mut text = String::from(cell.c);
                text.extend(&cell.zerowidth);
                let width = m.cell_w * if cell.wide { 2. } else { 1. };
                paint_glyph(text, text_run(cell, 0, fg_of(cell)), x, y, width, m.line_h, window, cx);
                col += 1;
            }
        }
    }

    if let Some(marked) = marked.filter(|s| !s.is_empty()) {
        paint_marked(snap, marked, origin, m, theme, window, cx);
    } else if let Some(cursor) = snap.cursor {
        let x = origin.x + m.cell_w * f32::from(cursor.col);
        let y = origin.y + m.line_h * f32::from(cursor.row);
        let w = m.cell_w * if cursor.wide { 2. } else { 1. };
        let accent = color(theme.term_palette.cursor, 1.0);
        match cursor.kind {
            CursorKind::Block => {
                window.paint_quad(fill(Bounds::new(point(x, y), size(w, m.line_h)), accent));
                // カーソルの上の文字を、背景の色で描き直す。
                if let Some(cell) = snap.lines.get(usize::from(cursor.row)).and_then(|l| l.get(usize::from(cursor.col)))
                    && !cell.c.is_whitespace()
                {
                    let mut text = String::from(cell.c);
                    text.extend(&cell.zerowidth);
                    let fg = color(theme.term_palette.background, 1.0);
                    paint_glyph(text, text_run(cell, 0, fg), x, y, w, m.line_h, window, cx);
                }
            }
            CursorKind::Beam => window.paint_quad(fill(Bounds::new(point(x, y), size(px(2.), m.line_h)), accent)),
            CursorKind::Underline => {
                window.paint_quad(fill(Bounds::new(point(x, y + m.line_h - px(2.)), size(w, px(2.))), accent))
            }
            CursorKind::Hollow => {
                window.paint_quad(outline(Bounds::new(point(x, y), size(w, m.line_h)), accent, BorderStyle::Solid))
            }
        }
    }
}

/// IME の変換中の文字列を、カーソルの位置から描く。
fn paint_marked(
    snap: &Snapshot,
    marked: &str,
    origin: Point<Pixels>,
    m: CellMetrics,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) {
    let (row, col) = snap.cursor.map_or((0, 0), |c| (c.row, c.col));
    let at = point(origin.x + m.cell_w * f32::from(col), origin.y + m.line_h * f32::from(row));
    let fg: Hsla = theme.fg_default.into();
    let run = TextRun {
        len: marked.len(),
        font: mono_font(),
        color: fg,
        background_color: None,
        underline: Some(UnderlineStyle { color: Some(theme.accent.into()), thickness: px(1.), wavy: false }),
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(marked.to_string().into(), TERM_FONT_SIZE, &[run], None);
    window.paint_quad(fill(Bounds::new(at, size(shaped.width, m.line_h)), theme.bg_panel));
    let _ = shaped.paint(at, m.line_h, TextAlign::Left, None, window, cx);
}
