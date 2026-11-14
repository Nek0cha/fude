//! Markdown の表示モードのための小さな計算（設計書 §5.4）。
//!
//! プレビュー（GPUI Kit の TextView）は、`markdown` クレートの mdast の最上位のブロックを 1 つずつ
//! リストの項目にする。ここでも同じクレートと同じ設定で最上位のブロックの開始行を出し、
//! ソースの行とプレビューの項目を対応させる。

use markdown::ParseOptions;
use markdown::mdast::Node;

/// 表示モードで、ソースとプレビューを左右に並べるエディタ列の最小の幅（論理ピクセル）。
pub const MD_SPLIT_MIN: f32 = 640.0;

/// Markdown のファイルか（`.md` / `.markdown`、大文字小文字を問わない）。
pub fn is_markdown(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// エディタ列の幅が column_w のとき、ソースとプレビューを並べるか（狭ければプレビューだけ）。
pub fn split_preview(column_w: f32) -> bool {
    column_w >= MD_SPLIT_MIN
}

/// GPUI Kit の TextView と同じ解析の設定（GFM と数式。frontmatter は既定で読まない）。
fn parse_options() -> ParseOptions {
    let mut options = ParseOptions::gfm();
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    options
}

/// mdast の最上位のブロックの開始行（0 始まり）。解析に失敗したら空。
pub fn block_lines(text: &str) -> Vec<u32> {
    let Ok(Node::Root(root)) = markdown::to_mdast(text, &parse_options()) else { return Vec::new() };
    root.children
        .iter()
        .filter_map(|n| n.position().map(|p| p.start.line.saturating_sub(1) as u32))
        .collect()
}

/// ブロック i の（開始行, 終わりの行）。終わりは次のブロックの開始行、最後は total_lines。
fn block_span(blocks: &[u32], total_lines: u32, i: usize) -> (f32, f32) {
    let start = blocks[i] as f32;
    let end = blocks.get(i + 1).copied().unwrap_or(total_lines).max(blocks[i]) as f32;
    (start, end)
}

/// ソースの先頭行（小数あり）→（プレビューの項目の番号, 項目の中の割合 0..1）。
/// ブロックの間の空行は前のブロックに含める。最初のブロックより前とブロックが無いときは (0, 0.0)。
pub fn source_to_preview(blocks: &[u32], total_lines: u32, top_line: f32) -> (usize, f32) {
    let Some(i) = blocks.iter().rposition(|&b| b as f32 <= top_line) else { return (0, 0.0) };
    let (start, end) = block_span(blocks, total_lines, i);
    let fraction = if end > start { ((top_line - start) / (end - start)).clamp(0.0, 1.0) } else { 0.0 };
    (i, fraction)
}

/// プレビューの（項目の番号, 項目の中の割合）→ ソースの行（小数あり）。ブロックが無ければ 0。
pub fn preview_to_source(blocks: &[u32], total_lines: u32, item: usize, fraction: f32) -> f32 {
    if blocks.is_empty() {
        return 0.0;
    }
    let (start, end) = block_span(blocks, total_lines, item.min(blocks.len() - 1));
    start + (end - start) * fraction.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_follow_mdast_top_level() {
        let text = "# A\n\npara\nline2\n\n- x\n- y\n\n> q\n";
        assert_eq!(block_lines(text), vec![0, 2, 5, 8]);
    }

    #[test]
    fn fenced_code_is_one_block() {
        let text = "# T\n\n```bash\n# not a heading\n- [ ] no\n```\n\nend\n";
        assert_eq!(block_lines(text), vec![0, 2, 7]);
    }

    /// GPUI Kit の既定は frontmatter を読まない。`---` は区切り線、続く 2 行は setext の見出しになる。
    #[test]
    fn crlf_and_frontmatter() {
        assert_eq!(block_lines("---\nt: 1\n---\n\n# H\r\n\r\np\r\n"), vec![0, 1, 4, 6]);
    }

    #[test]
    fn source_maps_to_block_and_fraction() {
        let blocks = [0, 2, 5];
        assert_eq!(source_to_preview(&blocks, 10, 0.0), (0, 0.0));
        assert_eq!(source_to_preview(&blocks, 10, 3.5), (1, 0.5));
        assert_eq!(source_to_preview(&blocks, 10, 9.0), (2, 0.8));
        assert_eq!(source_to_preview(&[], 10, 3.0), (0, 0.0));
    }

    #[test]
    fn preview_maps_back_to_line() {
        let blocks = [0, 2, 5];
        assert_eq!(preview_to_source(&blocks, 10, 1, 0.5), 3.5);
        assert_eq!(preview_to_source(&blocks, 10, 2, 1.0), 10.0);
        assert_eq!(preview_to_source(&[], 10, 0, 0.0), 0.0);
    }

    #[test]
    fn split_threshold() {
        assert!(split_preview(640.0));
        assert!(!split_preview(639.0));
        assert!(is_markdown("docs/A.MD") && is_markdown("x.markdown") && !is_markdown("x.mdx"));
    }
}
