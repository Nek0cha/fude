//! ターミナルの出力の中の、ファイルのパス（行と列つき）と URL を見つける（設計書 §5.2）。

use std::path::PathBuf;

use fude_core::location::FileLocation;
use fude_core::workspace::{ProjectRoot, ShellKind};

/// 見つけたリンク。start..end は行の文字列の char の位置（行と列の印を含む）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub target: LinkTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Url(String),
    Path { path: String, line: Option<u32>, col: Option<u32> },
}

/// 前に付く囲みの文字。
const OPEN: &[char] = &['(', '[', '{', '<', '"', '\'', '`'];
/// 後ろに付く囲みや句読点（行と列の印の ")" と ":" は別に扱う）。
const TRAIL: &[char] = &[']', '}', '>', '"', '\'', '`', ',', ';', '.', '!', '?'];

/// 行の文字列からリンクを探す。空白で区切った語ごとに、URL か、拡張子の付いたファイルのパス（`:行`、`:行:列`、
/// `(行,列)` が付いてよい）を拾う。空白を含むパスは拾わない。
pub fn find_links(text: &str) -> Vec<Link> {
    let chars: Vec<char> = text.chars().collect();
    let mut links = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        if let Some(link) = token_link(&chars[start..i], start) {
            links.push(link);
        }
    }
    links
}

fn token_link(token: &[char], offset: usize) -> Option<Link> {
    let mut s = 0;
    while s < token.len() && OPEN.contains(&token[s]) {
        s += 1;
    }
    let core: String = token[s..].iter().collect();
    if core.starts_with("http://") || core.starts_with("https://") {
        let url = core.trim_end_matches(|c: char| TRAIL.contains(&c) || c == ')' || c == ':');
        return Some(Link { start: offset + s, end: offset + s + url.chars().count(), target: LinkTarget::Url(url.to_string()) });
    }
    let mut core = core.trim_end_matches(|c: char| TRAIL.contains(&c)).to_string();
    let (mut line, mut col) = (None, None);
    // `path(行,列)` と `path(行)`。
    if core.ends_with(')')
        && let Some(open) = core.rfind('(')
    {
        let inner = &core[open + 1..core.len() - 1];
        let mut nums = inner.split(',').map(|n| n.trim().parse::<u32>());
        match (nums.next(), nums.next(), nums.next()) {
            (Some(Ok(l)), None, None) => (line, col) = (Some(l), None),
            (Some(Ok(l)), Some(Ok(c)), None) => (line, col) = (Some(l), Some(c)),
            _ => return None,
        }
        core.truncate(open);
    } else {
        core = core.trim_end_matches(')').trim_end_matches(':').to_string();
        // `path:行` と `path:行:列`（右から数字の欄だけを外す。ドライブ名の ":" は残る）。
        let mut nums = Vec::new();
        while let Some((head, tail)) = core.rsplit_once(':') {
            match tail.parse::<u32>() {
                Ok(n) if nums.len() < 2 && !tail.is_empty() => {
                    nums.push(n);
                    core = head.to_string();
                }
                _ => break,
            }
        }
        match nums.as_slice() {
            [l] => line = Some(*l),
            [c, l] => (line, col) = (Some(*l), Some(*c)),
            _ => {}
        }
    }
    if !looks_like_file(&core) {
        return None;
    }
    // 終わりの位置は、元の語の中で、パスと行・列の印が占める長さ。
    let consumed = consumed_len(&token[s..], &core, line, col);
    Some(Link { start: offset + s, end: offset + s + consumed, target: LinkTarget::Path { path: core, line, col } })
}

/// パスと行・列の印の、元の語の中での char の長さ。
fn consumed_len(rest: &[char], path: &str, line: Option<u32>, col: Option<u32>) -> usize {
    let mut n = path.chars().count();
    let tail: String = rest[n.min(rest.len())..].iter().collect();
    let mut suffixes = Vec::new();
    if let Some(l) = line {
        match col {
            Some(c) => {
                suffixes.push(format!(":{l}:{c}"));
                suffixes.push(format!("({l},{c})"));
            }
            None => {
                suffixes.push(format!(":{l}"));
                suffixes.push(format!("({l})"));
            }
        }
    }
    if let Some(sfx) = suffixes.iter().find(|s| tail.starts_with(s.as_str())) {
        n += sfx.chars().count();
    }
    n
}

/// 拡張子の付いたファイル名で終わるか。拡張子は英数字 1〜10 文字で英字を含む。"e.g" のような略語は除く。
fn looks_like_file(path: &str) -> bool {
    if path.is_empty() || path.chars().any(|c| matches!(c, '<' | '>' | '|' | '"' | '*' | '?') || c.is_control()) {
        return false;
    }
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let Some((stem, ext)) = name.rsplit_once('.') else { return false };
    if stem.is_empty() && !name.starts_with('.') {
        return false;
    }
    let ext_ok = (1..=10).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && ext.chars().any(|c| c.is_ascii_alphabetic());
    if !ext_ok {
        return false;
    }
    let has_sep = path.contains('/') || path.contains('\\');
    // 区切りの無い語で、点で分けた欄がすべて 1 文字なら略語（e.g、a.b.c）とみなす。
    !(!has_sep && name.split('.').all(|seg| seg.chars().count() <= 1))
}

fn is_windows_abs(path: &str) -> bool {
    let b = path.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || path.starts_with(r"\\")
}

/// リンクのパスを、開く場所にする。WSL のシェルの "/" 始まりは WSL のパス（/mnt/<ドライブ> は Windows）、
/// Windows の絶対パスと、ネイティブのシェルの絶対パスはこのマシンのパス、相対パスはプロジェクトのルートから。
/// ".." を含む相対パスは None（プロジェクトの外へは出ない）。
pub fn resolve_link(path: &str, kind: &ShellKind, root: &ProjectRoot) -> Option<FileLocation> {
    if let ShellKind::Wsl { distro } = kind
        && path.starts_with('/')
    {
        return Some(FileLocation::from_wsl_path(distro, path));
    }
    if is_windows_abs(path) || (path.starts_with('/') && !matches!(kind, ShellKind::Wsl { .. })) {
        return Some(FileLocation::from_host_path(PathBuf::from(path)));
    }
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".").collect();
    if parts.is_empty() || parts.contains(&"..") {
        return None;
    }
    match root {
        ProjectRoot::Local(dir) => {
            let mut p = dir.clone();
            for part in &parts {
                p.push(part);
            }
            Some(FileLocation::Local(p))
        }
        ProjectRoot::Wsl { distro, path: dir } => {
            let base = dir.trim_end_matches('/');
            Some(FileLocation::Wsl { distro: distro.clone(), path: format!("{base}/{}", parts.join("/")) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fude_core::location::FileLocation;
    use fude_core::workspace::{ProjectRoot, ShellKind};
    use std::path::PathBuf;

    fn targets(links: &[Link]) -> Vec<LinkTarget> {
        links.iter().map(|l| l.target.clone()).collect()
    }

    fn path(p: &str, line: Option<u32>, col: Option<u32>) -> LinkTarget {
        LinkTarget::Path { path: p.into(), line, col }
    }

    #[test]
    fn finds_paths_with_line_and_col() {
        let text = "error at src/main.rs:42:7 and (crates/a b.rs) see https://example.com/x.";
        let links = find_links(text);
        assert_eq!(
            targets(&links),
            vec![path("src/main.rs", Some(42), Some(7)), path("b.rs", None, None), LinkTarget::Url("https://example.com/x".into())]
        );
        let chars: Vec<char> = text.chars().collect();
        let first: String = chars[links[0].start..links[0].end].iter().collect();
        assert_eq!(first, "src/main.rs:42:7");
    }

    #[test]
    fn finds_windows_and_wsl_paths() {
        let links = find_links(r"C:\w\a.rs:3 /home/x/p/b.ts .\c.md src\d.rs(10,2) 'q.toml'");
        assert_eq!(
            targets(&links),
            vec![
                path(r"C:\w\a.rs", Some(3), None),
                path("/home/x/p/b.ts", None, None),
                path(r".\c.md", None, None),
                path(r"src\d.rs", Some(10), Some(2)),
                path("q.toml", None, None),
            ]
        );
    }

    #[test]
    fn ignores_plain_words_and_versions() {
        assert!(find_links("v1.2.3 e.g. foo 3.14 crates/dir a.b.c").is_empty());
    }

    #[test]
    fn char_positions_after_wide_chars() {
        let text = "日本語 src/a.rs:1";
        let links = find_links(text);
        assert_eq!(links[0].start, 4);
        assert_eq!(links[0].end, text.chars().count());
    }

    #[test]
    fn resolve_link_rules() {
        let ubuntu = ShellKind::Wsl { distro: "Ubuntu".into() };
        let wsl_root = ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x/p".into() };
        let local_root = ProjectRoot::Local(PathBuf::from("/w/app"));
        assert_eq!(
            resolve_link("/home/x/p/a.rs", &ubuntu, &wsl_root),
            Some(FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/x/p/a.rs".into() })
        );
        assert_eq!(resolve_link("/mnt/c/w/a.rs", &ubuntu, &wsl_root), Some(FileLocation::Local(PathBuf::from(r"C:\w\a.rs"))));
        assert_eq!(
            resolve_link("./src/a.rs", &ubuntu, &wsl_root),
            Some(FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/x/p/src/a.rs".into() })
        );
        assert_eq!(
            resolve_link("src/a.rs", &ShellKind::Native, &local_root),
            Some(FileLocation::Local(PathBuf::from("/w/app").join("src").join("a.rs")))
        );
        assert_eq!(resolve_link("../x.rs", &ShellKind::Native, &local_root), None);
    }
}
