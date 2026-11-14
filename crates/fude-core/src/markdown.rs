//! Markdown の表示モードのための小さな計算（設計書 §5.4）。
//!
//! プレビュー（GPUI Kit の TextView）は、`markdown` クレートの mdast の最上位のブロックを 1 つずつ
//! リストの項目にする。ここでも同じクレートと同じ設定で最上位のブロックの開始行を出し、
//! ソースの行とプレビューの項目を対応させる。

use markdown::ParseOptions;
use markdown::mdast::Node;

use crate::location::{FileLocation, ProjectRoot};

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

/// プレビューとドロップで画像として扱う拡張子。
const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "svg"];

/// 画像のファイルか（拡張子で決める。大文字小文字を問わない）。
pub fn is_image(rel: &str) -> bool {
    let name = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    name.rsplit_once('.').is_some_and(|(_, ext)| IMAGE_EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

/// Markdown に書く画像の行き先。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// プロジェクトのルートからの相対パス（"/" 区切り）。
    Project(String),
    /// プロジェクトの外の絶対パス（"/" にそろえたもの）。
    Outside(String),
}

/// ファイルが root の中なら相対パス、外なら "/" にそろえた絶対パス（WSL は `//wsl.localhost/<d>/…`）。
pub fn link_target(root: &ProjectRoot, file: &FileLocation) -> LinkTarget {
    if let Some(rel) = file.rel_within(root) {
        return LinkTarget::Project(rel);
    }
    match file {
        FileLocation::Local(p) => LinkTarget::Outside(p.to_string_lossy().replace('\\', "/")),
        FileLocation::Wsl { distro, path } => LinkTarget::Outside(format!("//wsl.localhost/{distro}{path}")),
    }
}

/// md_rel のフォルダの要素（ファイル名を除く）。
fn md_dir(md_rel: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = md_rel.split('/').filter(|s| !s.is_empty()).collect();
    parts.pop();
    parts
}

/// md_rel のフォルダから、ルートからの相対パス target への相対パス。
fn relative_from_md(md_rel: &str, target: &str) -> String {
    let dir = md_dir(md_rel);
    let to: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    let limit = dir.len().min(to.len().saturating_sub(1));
    let common = (0..limit).take_while(|&i| dir[i] == to[i]).count();
    let mut out = "../".repeat(dir.len() - common);
    out.push_str(&to[common..].join("/"));
    out
}

/// `![](…)` の行（改行は付けない）。空白、括弧、`<`、`>` を含む行き先は `<…>` で囲む。
pub fn image_link(md_rel: &str, target: &LinkTarget) -> String {
    let dest = match target {
        LinkTarget::Project(rel) => relative_from_md(md_rel, rel),
        LinkTarget::Outside(abs) => abs.clone(),
    };
    // URL として読まれる `%`、`#`、`?` は符号化する（読むときは resolve_image が戻す）。
    let dest = dest.replace('%', "%25").replace('#', "%23").replace('?', "%3F");
    if dest.chars().any(|c| c.is_whitespace() || "()<>".contains(c)) { format!("![](<{dest}>)") } else { format!("![]({dest})") }
}

/// プレビューの画像の場所。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageRef {
    /// http(s) と data: の URL。そのまま渡す。
    Remote(String),
    /// ルートからの相対パス。ルートの外へ出る `../` は残る。
    ProjectRel(String),
    /// 絶対パス（"/" か "ドライブ名:" で始まる）。
    Absolute(String),
}

/// プレビューのリンクの行き先。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewLink {
    /// ブラウザで開く。
    External(String),
    /// ルートからの相対パス。エディタ列で開く。
    Project(String),
}

fn has_scheme(url: &str, schemes: &[&str]) -> bool {
    let lower = url.to_ascii_lowercase();
    schemes.iter().any(|s| lower.starts_with(s))
}

fn is_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/') || path.starts_with('\\') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// `%XX` を戻す（UTF-8 として不正なら元のまま）。
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// URL の `?` と `#` 以降を捨て、`%XX` を戻す。
fn clean_path(url: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    percent_decode(&url[..end])
}

/// md_rel のフォルダに path をつなぎ、`.` と `..` をたたむ。ルートより上の `..` は残す。
fn join_from_md(md_rel: &str, path: &str) -> String {
    let mut stack: Vec<&str> = md_dir(md_rel);
    for part in path.split('/').filter(|s| !s.is_empty() && *s != ".") {
        if part == ".." && stack.last().is_some_and(|t| *t != "..") {
            stack.pop();
        } else {
            stack.push(part);
        }
    }
    stack.join("/")
}

/// プレビューの画像の url を場所にする。空なら None。
pub fn resolve_image(md_rel: &str, url: &str) -> Option<ImageRef> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if has_scheme(url, &["http://", "https://", "data:"]) {
        return Some(ImageRef::Remote(url.to_string()));
    }
    let path = clean_path(url);
    if path.is_empty() {
        return None;
    }
    if is_absolute(&path) {
        return Some(ImageRef::Absolute(path));
    }
    Some(ImageRef::ProjectRel(join_from_md(md_rel, &path)))
}

/// プレビューのリンクの url を行き先にする。ページ内のリンク、絶対パス、ルートの外は None。
pub fn resolve_link(md_rel: &str, url: &str) -> Option<PreviewLink> {
    let url = url.trim();
    if has_scheme(url, &["http://", "https://", "mailto:"]) {
        return Some(PreviewLink::External(url.to_string()));
    }
    let path = clean_path(url);
    if path.is_empty() || is_absolute(&path) {
        return None;
    }
    let rel = join_from_md(md_rel, &path);
    if rel.is_empty() || rel.starts_with("..") {
        return None;
    }
    Some(PreviewLink::Project(rel))
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

/// ブロックの数がプレビューの項目の数と合わないときの写し方。全体の割合 ratio（0..1）→（項目の番号, 項目の中の割合）。
pub fn proportional(ratio: f32, items: usize) -> (usize, f32) {
    if items == 0 {
        return (0, 0.0);
    }
    let at = ratio.clamp(0.0, 1.0) * items as f32;
    if at >= items as f32 {
        return (items - 1, 1.0);
    }
    (at.trunc() as usize, at.fract())
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

    #[test]
    fn image_links_relative_to_md() {
        let p = |s: &str| LinkTarget::Project(s.into());
        assert_eq!(image_link("docs/a.md", &p("docs/img/x.png")), "![](img/x.png)");
        assert_eq!(image_link("docs/a.md", &p("assets/x.png")), "![](../assets/x.png)");
        assert_eq!(image_link("a.md", &p("my pic (1).png")), "![](<my pic (1).png>)");
        assert_eq!(image_link("a.md", &LinkTarget::Outside("C:/Users/me/x.png".into())), "![](C:/Users/me/x.png)");
    }

    #[test]
    fn link_round_trips_through_resolver() {
        for target in ["docs/img/x.png", "assets/a b.png", "x.png"] {
            let link = image_link("docs/a.md", &LinkTarget::Project(target.into()));
            let url = link.trim_start_matches("![](").trim_end_matches(')').trim_start_matches('<').trim_end_matches('>');
            assert_eq!(resolve_image("docs/a.md", url), Some(ImageRef::ProjectRel(target.into())));
        }
    }

    #[test]
    fn link_target_inside_and_outside() {
        use crate::location::{FileLocation, ProjectRoot};
        let root = ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/me/p".into() };
        let inside = FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/me/p/img/a.png".into() };
        assert_eq!(link_target(&root, &inside), LinkTarget::Project("img/a.png".into()));
        let outside = FileLocation::Wsl { distro: "Ubuntu".into(), path: "/tmp/a.png".into() };
        assert_eq!(link_target(&root, &outside), LinkTarget::Outside("//wsl.localhost/Ubuntu/tmp/a.png".into()));
        let win = FileLocation::Local(std::path::PathBuf::from(r"C:\Users\me\a.png"));
        assert_eq!(link_target(&root, &win), LinkTarget::Outside("C:/Users/me/a.png".into()));
    }

    #[test]
    fn resolver_kinds() {
        assert_eq!(resolve_image("a.md", "https://x/y.png"), Some(ImageRef::Remote("https://x/y.png".into())));
        assert_eq!(resolve_image("a.md", "C:/p/x.png"), Some(ImageRef::Absolute("C:/p/x.png".into())));
        assert_eq!(resolve_image("a.md", "/home/x.png"), Some(ImageRef::Absolute("/home/x.png".into())));
        assert_eq!(resolve_image("d/a.md", "i%20j.png?raw=1#t"), Some(ImageRef::ProjectRel("d/i j.png".into())));
        assert_eq!(resolve_image("a.md", ""), None);
    }

    #[test]
    fn preview_links() {
        assert_eq!(resolve_link("d/a.md", "b.md#sec"), Some(PreviewLink::Project("d/b.md".into())));
        assert_eq!(resolve_link("a.md", "https://x"), Some(PreviewLink::External("https://x".into())));
        assert_eq!(resolve_link("a.md", "../out.md"), None);
    }

    #[test]
    fn images_by_extension() {
        assert!(is_image("a.PNG") && is_image("b.svg") && is_image("c.webp") && !is_image("d.md"));
    }

    #[test]
    fn proportional_fallback() {
        assert_eq!(proportional(0.25, 8), (2, 0.0));
        assert_eq!(proportional(1.0, 8), (7, 1.0));
        assert_eq!(proportional(0.5, 0), (0, 0.0));
    }

    /// `#`、`?`、`%` を含む名前も、書いたリンクから同じファイルに戻る。
    #[test]
    fn link_escapes_url_specials() {
        assert_eq!(image_link("a.md", &LinkTarget::Project("issue#12.png".into())), "![](issue%2312.png)");
        for target in ["issue#12.png", "q?.png", "a%20b.png", "d/50% off #1.png"] {
            let link = image_link("a.md", &LinkTarget::Project(target.into()));
            let url = link.trim_start_matches("![](").trim_end_matches(')').trim_start_matches('<').trim_end_matches('>');
            assert_eq!(resolve_image("a.md", url), Some(ImageRef::ProjectRel(target.into())), "{link}");
        }
    }
}
