//! 全体検索と置換（設計書 §5.6）。gitignore を尊重して探す。WSL のプロジェクトでは fude serve の中で同じ処理を走らせる。

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{ProjectFs, RelPath};

/// 検索の条件。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub pattern: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// 1 行の一致。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineMatch {
    /// 1 始まりの行。
    pub line: u32,
    /// 1 始まりの、最初の一致の文字（char）の位置。元の行で数える。
    pub col: u32,
    /// 表示用の行（\r\n を除き、長ければ切り出したもの）。
    pub text: String,
    /// text の中の一致のバイト範囲（空の範囲は含めない）。
    pub ranges: Vec<(u32, u32)>,
}

/// 1 ファイルの一致。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMatches {
    pub rel: RelPath,
    pub lines: Vec<LineMatch>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    pub files: Vec<FileMatches>,
    /// 件数か時間の上限で打ち切った。
    pub truncated: bool,
}

/// 一致の数の上限（全体）。
pub const SEARCH_MAX_MATCHES: usize = 10_000;
/// 検索にかける時間の上限。fude serve の応答待ち（10 秒）より短くする。
pub const SEARCH_TIME_LIMIT: Duration = Duration::from_secs(8);
/// 行のプレビューの文字数の上限。
pub const PREVIEW_MAX_CHARS: usize = 240;

/// プレビューで、最初の一致の前に残す文字数。
const PREVIEW_LEAD_CHARS: usize = 40;

/// 検索と置換の両方で使う正規表現の文字列。空の pattern、正しくない正規表現 → Err(理由の文)。
pub fn build_pattern(q: &SearchQuery) -> Result<String, String> {
    if q.pattern.is_empty() {
        return Err("検索する文字列が空です".into());
    }
    let body = if q.regex { q.pattern.clone() } else { regex::escape(&q.pattern) };
    let pattern = if q.whole_word { format!(r"\b(?:{body})\b") } else { body };
    compile(&pattern, q)?;
    Ok(pattern)
}

/// 検索の条件の正規表現（置換のプレビューで $1 を展開するのに使う）。
pub fn compile_query(q: &SearchQuery) -> Result<regex::Regex, String> {
    compile(&build_pattern(q)?, q)
}

fn compile(pattern: &str, q: &SearchQuery) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(pattern).case_insensitive(!q.case_sensitive).build().map_err(|e| e.to_string())
}

fn invalid_input(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// 行末の \r\n / \n を除く。
fn trim_eol(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// 1 行の一致を作る。空でない一致が無ければ None。
fn line_match(re: &regex::Regex, line_no: u64, line: &str) -> Option<LineMatch> {
    let ranges: Vec<(usize, usize)> = re.find_iter(line).filter(|m| !m.is_empty()).map(|m| (m.start(), m.end())).collect();
    let first = ranges.first()?.0;
    let col = line[..first].chars().count() as u32 + 1;
    let (text, ranges) = preview(line, &ranges, first);
    Some(LineMatch { line: line_no as u32, col, text, ranges })
}

/// 長い行は、最初の一致の少し前から PREVIEW_MAX_CHARS 文字だけを切り出す。範囲は切り出した文字列に合わせる。
fn preview(line: &str, ranges: &[(usize, usize)], first: usize) -> (String, Vec<(u32, u32)>) {
    if line.chars().count() <= PREVIEW_MAX_CHARS {
        return (line.to_string(), ranges.iter().map(|&(s, e)| (s as u32, e as u32)).collect());
    }
    let first_char = line[..first].chars().count();
    let start_char = first_char.saturating_sub(PREVIEW_LEAD_CHARS);
    let byte_at = |n: usize| line.char_indices().nth(n).map_or(line.len(), |(i, _)| i);
    let start = byte_at(start_char);
    let end = byte_at(start_char + PREVIEW_MAX_CHARS);
    let prefix = if start > 0 { "…" } else { "" };
    let shift = prefix.len() as i64 - start as i64;
    let text = format!("{prefix}{}", &line[start..end]);
    let ranges = ranges
        .iter()
        .filter(|&&(s, e)| s >= start && e <= end)
        .map(|&(s, e)| ((s as i64 + shift) as u32, (e as i64 + shift) as u32))
        .collect();
    (text, ranges)
}

/// gitignore を尊重して root の下を探す（.git を除く、隠しファイルは含む、バイナリは飛ばす）。rel の名前順。
pub fn search_dir(root: &Path, q: &SearchQuery) -> io::Result<SearchResults> {
    let pattern = build_pattern(q).map_err(invalid_input)?;
    let re = compile(&pattern, q).map_err(invalid_input)?;
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .case_insensitive(!q.case_sensitive)
        .line_terminator(Some(b'\n'))
        // CRLF のファイルでも $ が \r の前で一致するように。
        .crlf(true)
        .multi_line(true)
        .build(&pattern)
        .map_err(|e| invalid_input(e.to_string()))?;
    let mut searcher = grep_searcher::SearcherBuilder::new()
        .line_number(true)
        // 一致させる側の crlf(true) と合わせる（合わないと検索がエラーになる）。LF だけの行も行として扱う。
        .line_terminator(grep_matcher::LineTerminator::crlf())
        .binary_detection(grep_searcher::BinaryDetection::quit(0))
        .build();
    let started = Instant::now();
    let canonical_root = std::fs::canonicalize(root).ok();
    let walker =
        ignore::WalkBuilder::new(root).hidden(false).parents(true).filter_entry(|e| e.file_name() != ".git").build();
    let mut results = SearchResults::default();
    let mut total = 0usize;
    for entry in walker {
        if started.elapsed() >= SEARCH_TIME_LIMIT || total >= SEARCH_MAX_MATCHES {
            results.truncated = true;
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file() || (t.is_symlink() && entry.path().is_file())) {
            continue;
        }
        // ルートの中のファイルを指すリンク（CLAUDE.md -> AGENTS.md など）は、指す先だけを出す。
        // 両方を出すと、置換が同じ中身に 2 回当たる。
        if entry.path_is_symlink()
            && let (Some(croot), Ok(target)) = (&canonical_root, std::fs::canonicalize(entry.path()))
            && target.starts_with(croot)
        {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue };
        let Ok(rel) = RelPath::new(&rel.to_string_lossy()) else { continue };
        let mut lines = Vec::new();
        let sink = grep_searcher::sinks::Bytes(|line_no, bytes| {
            let text = String::from_utf8_lossy(bytes);
            if let Some(m) = line_match(&re, line_no, trim_eol(&text)) {
                lines.push(m);
                total += 1;
            }
            Ok(total < SEARCH_MAX_MATCHES)
        });
        // 読めないファイルは飛ばす。
        if searcher.search_path(&matcher, entry.path(), sink).is_err() {
            continue;
        }
        if total >= SEARCH_MAX_MATCHES {
            results.truncated = true;
        }
        if !lines.is_empty() {
            results.files.push(FileMatches { rel, lines });
        }
    }
    results.files.sort_by(|a, b| a.rel.as_str().cmp(b.rel.as_str()));
    Ok(results)
}

/// text の一致を replacement に置き換える。戻り値は（新しい文字列, 置き換えた数）。
/// 正規表現のときは $1 などを展開し、そうでなければ文字どおりに入れる。検索と同じく 1 行ずつ当て、行をまたがない。
pub fn replace_text(text: &str, q: &SearchQuery, replacement: &str) -> Result<(String, usize), String> {
    let re = compile(&build_pattern(q)?, q)?;
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    for line in text.split_inclusive('\n') {
        let body = trim_eol(line);
        let eol = &line[body.len()..];
        // 空の一致は検索に出さないので、置換もしない（プレビューと同じ所だけを変える）。
        let mut at = 0;
        for caps in re.captures_iter(body) {
            let m = caps.get(0).expect("一致の全体");
            if m.is_empty() {
                continue;
            }
            out.push_str(&body[at..m.start()]);
            if q.regex {
                caps.expand(replacement, &mut out);
            } else {
                out.push_str(replacement);
            }
            at = m.end();
            count += 1;
        }
        out.push_str(&body[at..]);
        out.push_str(eol);
    }
    Ok((out, count))
}

/// fs の rel を読み、replace_text して、1 件以上なら書く。UTF-8 でなければ InvalidData（黙って飛ばさない）。
pub fn replace_in_file(fs: &dyn ProjectFs, rel: &RelPath, q: &SearchQuery, replacement: &str) -> io::Result<usize> {
    let data = fs.read(rel)?;
    let Ok(text) = String::from_utf8(data) else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "UTF-8 ではないため置換できません"));
    };
    let (new, count) = replace_text(&text, q, replacement).map_err(invalid_input)?;
    if count > 0 {
        fs.write(rel, new.as_bytes())?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::{
        LineMatch, PREVIEW_MAX_CHARS, SEARCH_MAX_MATCHES, SearchQuery, build_pattern, replace_in_file, replace_text, search_dir,
    };
    use crate::{LocalFs, RelPath};
    use std::io;
    use std::path::Path;

    fn q(s: &str) -> SearchQuery {
        SearchQuery { pattern: s.into(), ..Default::default() }
    }

    fn rel(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    fn write(root: &Path, name: &str, data: &[u8]) {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }

    fn count(root: &Path, q: SearchQuery) -> usize {
        search_dir(root, &q).unwrap().files.iter().flat_map(|f| &f.lines).map(|l| l.ranges.len()).sum()
    }

    #[test]
    fn literal_search_is_case_insensitive_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", b"Foo\nbar foo\n");
        write(root, "b.txt", b"nothing\n");
        let r = search_dir(root, &q("foo")).unwrap();
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].rel.as_str(), "a.txt");
        let lines: Vec<(u32, u32)> = r.files[0].lines.iter().map(|l| (l.line, l.col)).collect();
        assert_eq!(lines, [(1, 1), (2, 5)]);
        assert_eq!(r.files[0].lines[1].ranges, [(4, 7)]);
        assert!(!r.truncated);
    }

    #[test]
    fn case_word_and_regex_toggles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", b"foo food Foo\n");
        assert_eq!(count(root, SearchQuery { pattern: "Foo".into(), case_sensitive: true, ..Default::default() }), 1);
        assert_eq!(count(root, SearchQuery { pattern: "foo".into(), whole_word: true, ..Default::default() }), 2);
        assert_eq!(count(root, SearchQuery { pattern: "fo+d".into(), regex: true, ..Default::default() }), 1);
        // 正規表現でないときは、記号を文字どおりに探す。
        assert_eq!(count(root, q("fo+d")), 0);
    }

    #[test]
    fn respects_gitignore_and_skips_binary() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // ignore クレートは .git が無いと .gitignore を読まないため、リポジトリの形にする。
        std::fs::create_dir_all(root.join(".git")).unwrap();
        write(root, ".gitignore", b"ignored/\n");
        write(root, "ignored/x.txt", b"foo\n");
        write(root, "bin.dat", b"foo\0foo\n");
        write(root, ".git/x", b"foo\n");
        write(root, ".hidden", b"foo\n");
        let rels: Vec<String> =
            search_dir(root, &q("foo")).unwrap().files.iter().map(|f| f.rel.as_str().to_string()).collect();
        assert_eq!(rels, [".hidden"]);
    }

    #[test]
    fn invalid_regex_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bad = SearchQuery { pattern: "(".into(), regex: true, ..Default::default() };
        assert!(build_pattern(&bad).is_err());
        assert_eq!(search_dir(root, &bad).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert!(replace_text("x", &bad, "y").is_err());
        assert!(build_pattern(&q("")).is_err());
    }

    #[test]
    fn empty_matches_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", b"abc\n");
        // "x*" はどこでも空に一致するが、空の範囲は結果に出さない。
        let r = search_dir(root, &SearchQuery { pattern: "x*".into(), regex: true, ..Default::default() }).unwrap();
        assert!(r.files.is_empty());
    }

    #[test]
    fn replace_with_empty_match_terminates() {
        // 空の一致は検索に出さないので、置換もしない（プレビューと同じ所だけを変える）。
        let q = SearchQuery { pattern: "x*".into(), regex: true, ..Default::default() };
        assert_eq!(replace_text("ab", &q, "-").unwrap(), ("ab".into(), 0));
        assert_eq!(replace_text("axxb", &q, "-").unwrap(), ("a-b".into(), 1));
        let d = SearchQuery { pattern: r"(\d)*".into(), regex: true, ..Default::default() };
        assert_eq!(replace_text("a1b\nxyz\n", &d, "N$1").unwrap(), ("aN1b\nxyz\n".into(), 1));
    }

    #[test]
    fn dollar_anchor_matches_in_crlf_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "crlf.txt", b"foo\r\n");
        write(root, "lf.txt", b"foo\n");
        let re = SearchQuery { pattern: "foo$".into(), regex: true, ..Default::default() };
        assert_eq!(search_dir(root, &re).unwrap().files.len(), 2);
    }

    #[test]
    fn replace_in_non_utf8_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "sjis.txt", b"foo \x82\xa0\n");
        let fs = LocalFs::new(dir.path().to_path_buf());
        let err = replace_in_file(&fs, &rel("sjis.txt"), &q("foo"), "bar").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_file_in_root_is_listed_once() {
        // CLAUDE.md -> AGENTS.md の形。両方を出すと、置換が同じ中身に 2 回当たる。
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "AGENTS.md", b"foo\n");
        std::os::unix::fs::symlink("AGENTS.md", root.join("CLAUDE.md")).unwrap();
        let rels: Vec<String> = search_dir(root, &q("foo")).unwrap().files.iter().map(|f| f.rel.as_str().to_string()).collect();
        assert_eq!(rels, ["AGENTS.md"]);
    }

    #[test]
    fn replace_literal_does_not_expand_captures() {
        assert_eq!(replace_text("a.b a.b", &q("a.b"), "$1").unwrap(), ("$1 $1".into(), 2));
        let re = SearchQuery { pattern: r"(\w+)@(\w+)".into(), regex: true, ..Default::default() };
        assert_eq!(replace_text("me@host", &re, "$2:$1").unwrap(), ("host:me".into(), 1));
    }

    #[test]
    fn crlf_lines_are_trimmed_and_kept_on_replace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", b"foo\r\nbar\r\n");
        let r = search_dir(root, &q("foo")).unwrap();
        assert_eq!(r.files[0].lines[0].text, "foo");
        assert_eq!(replace_text("foo\r\nbar\r\n", &q("foo"), "baz").unwrap().0, "baz\r\nbar\r\n");
    }

    #[test]
    fn long_line_preview_is_windowed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let line = format!("{}needle{}\n", "a".repeat(1000), "b".repeat(1000));
        write(root, "a.txt", line.as_bytes());
        let r = search_dir(root, &q("needle")).unwrap();
        let l: &LineMatch = &r.files[0].lines[0];
        // 先頭の "…" の分だけ多くてよい。
        assert!(l.text.chars().count() <= PREVIEW_MAX_CHARS + 1);
        assert!(l.text.starts_with('…'));
        let (s, e) = l.ranges[0];
        assert_eq!(&l.text[s as usize..e as usize], "needle");
        assert_eq!(l.col, 1001);
    }

    #[test]
    fn stops_at_max_matches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", "x\n".repeat(SEARCH_MAX_MATCHES + 5).as_bytes());
        let r = search_dir(root, &q("x")).unwrap();
        assert_eq!(r.files[0].lines.len(), SEARCH_MAX_MATCHES);
        assert!(r.truncated);
    }

    #[test]
    fn replace_in_file_writes_only_when_matched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt", b"foo\nfoo x\n");
        let fs = LocalFs::new(root.to_path_buf());
        assert_eq!(replace_in_file(&fs, &rel("a.txt"), &q("foo"), "bar").unwrap(), 2);
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "bar\nbar x\n");
        assert_eq!(replace_in_file(&fs, &rel("a.txt"), &q("zzz"), "y").unwrap(), 0);
        // UTF-8 でないファイルは触らず、置換できなかったことを返す。
        write(root, "s.txt", b"foo\xff");
        assert!(replace_in_file(&fs, &rel("s.txt"), &q("foo"), "y").is_err());
        assert_eq!(std::fs::read(root.join("s.txt")).unwrap(), b"foo\xff");
    }
}
