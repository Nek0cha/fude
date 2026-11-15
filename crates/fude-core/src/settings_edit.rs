//! 設定ファイルの 1 項目を書き換える（設計書 `2026-11-15-config-theme-design.md` §5.5）。
//! ほかの行、コメント、空行、改行の種類はそのまま残す。

use toml_edit::{DocumentMut, Item, Table, Value};

use crate::settings::{ParseOutcome, parse};

/// 書き込む値。
#[derive(Clone, Debug, PartialEq)]
pub enum EditValue {
    Bool(bool),
    Float(f64),
    Int(i64),
    Str(String),
    StrList(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// 今のファイルが TOML として読めない。手書きの内容を消さないため、書き込まない。
    Unreadable,
    /// 書き込むと読めないファイルになる（想定していない書き方）。書き込まない。
    WouldBreak,
}

impl EditValue {
    fn to_value(&self) -> Value {
        match self {
            EditValue::Bool(b) => Value::from(*b),
            EditValue::Float(f) => Value::from(*f),
            EditValue::Int(i) => Value::from(*i),
            EditValue::Str(s) => Value::from(s.as_str()),
            EditValue::StrList(list) => Value::Array(list.iter().map(String::as_str).collect()),
        }
    }
}

/// text の path（`["window", "background_opacity"]` など）の値を value にした文面を返す。
/// 項目があれば値だけを置き換える（行末のコメントは残す）。無ければ行を足す: 同じ項目のコメントアウトされた行が
/// その表にあればその直後、無ければ表の最後の行の後。表が無ければ文書の末尾に表を作る。
pub fn set_value(text: &str, path: &[&str], value: EditValue) -> Result<String, EditError> {
    if matches!(parse(text), ParseOutcome::Unreadable(_)) {
        return Err(EditError::Unreadable);
    }
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let mut doc: DocumentMut = body.parse().map_err(|_| EditError::Unreadable)?;
    let (key, tables) = path.split_last().expect("path は空でない");
    let existing = table_mut(doc.as_table_mut(), tables).and_then(|t| t.get_mut(key)).and_then(Item::as_value_mut);
    if let Some(v) = existing {
        let decor = v.decor().clone();
        *v = value.to_value();
        *v.decor_mut() = decor;
        // toml_edit は改行を LF で書き出すので、元が CRLF なら揃え直す。
        let mut out = doc.to_string();
        if body.contains("\r\n") {
            out = out.replace("\r\n", "\n").replace('\n', "\r\n");
        }
        return Ok(format!("{bom}{out}"));
    }
    let header = tables.join(".");
    let header_written = tables.is_empty() || body.lines().any(|l| header_name(l) == Some(header.as_str()));
    let out = if !header_written && item_mut(doc.as_item_mut(), tables).is_some() {
        // インライン表、ドット付きのキー、見出しの無い親の表: 行を足さず toml_edit で入れる。
        insert_with_toml_edit(&mut doc, tables, key, value.to_value());
        let mut out = doc.to_string();
        if body.contains("\r\n") {
            out = out.replace("\r\n", "\n").replace('\n', "\r\n");
        }
        out
    } else {
        insert_line(body, tables, key, &value.to_value())
    };
    if matches!(parse(&out), ParseOutcome::Unreadable(_)) {
        return Err(EditError::WouldBreak);
    }
    Ok(format!("{bom}{out}"))
}

/// text の path の項目を消した文面を返す（「自動」「テーマの色に戻す」のため）。行ごと消し、ほかの行とコメントは残す。
/// 項目が無ければ元のまま。インライン表やドット付きのキーは toml_edit で消す。
pub fn remove_value(text: &str, path: &[&str]) -> Result<String, EditError> {
    if matches!(parse(text), ParseOutcome::Unreadable(_)) {
        return Err(EditError::Unreadable);
    }
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let (key, tables) = path.split_last().expect("path は空でない");
    let mut doc: DocumentMut = body.parse().map_err(|_| EditError::Unreadable)?;
    let Some(item) = item_mut(doc.as_item_mut(), tables) else {
        return Ok(text.to_string());
    };
    let Some(table) = item.as_table_like_mut() else { return Ok(text.to_string()) };
    if table.get(key).is_none() {
        return Ok(text.to_string());
    }
    // 表の見出しの下に `key = ...` の 1 行で書いてあれば、その行だけを消す（前のコメントを残すため）。
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let header = tables.join(".");
    let mut in_table = tables.is_empty();
    let mut found = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(name) = header_name(line) {
            in_table = name == header;
            continue;
        }
        let is_key = line.trim_start().strip_prefix(key).is_some_and(|r| r.trim_start().starts_with('='));
        if in_table && is_key {
            found = Some(i);
            break;
        }
    }
    let out = match found {
        Some(i) => lines.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, l)| *l).collect::<String>(),
        None => {
            table.remove(key);
            let mut out = doc.to_string();
            if body.contains("\r\n") {
                out = out.replace("\r\n", "\n").replace('\n', "\r\n");
            }
            out
        }
    };
    if matches!(parse(&out), ParseOutcome::Unreadable(_)) {
        return Err(EditError::WouldBreak);
    }
    Ok(format!("{bom}{out}"))
}

/// names の先の項目（表かインライン表）。Item::get_mut は無いキーを作ってしまうので、TableLike で探す。
fn item_mut<'a>(mut item: &'a mut Item, names: &[&str]) -> Option<&'a mut Item> {
    for name in names {
        item = item.as_table_like_mut()?.get_mut(name)?;
    }
    item.is_table_like().then_some(item)
}

fn insert_with_toml_edit(doc: &mut DocumentMut, tables: &[&str], key: &str, value: Value) {
    let Some(item) = item_mut(doc.as_item_mut(), tables) else { return };
    match item {
        Item::Table(t) => {
            t.insert(key, Item::Value(value));
        }
        Item::Value(Value::InlineTable(t)) => {
            t.insert(key, value);
        }
        _ => {}
    }
}

fn table_mut<'a>(mut table: &'a mut Table, names: &[&str]) -> Option<&'a mut Table> {
    for name in names {
        table = table.get_mut(name)?.as_table_mut()?;
    }
    Some(table)
}

/// `[名前]` の見出しなら名前。
fn header_name(line: &str) -> Option<&str> {
    let t = line.trim();
    let inner = t.strip_prefix('[')?.split(']').next()?;
    Some(inner.trim().trim_matches('"'))
}

/// `# key = ...` の形のコメント行か。
fn is_commented_key(line: &str, key: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix('#') else { return false };
    rest.trim_start().strip_prefix(key).is_some_and(|r| r.trim_start().starts_with('='))
}

fn insert_line(text: &str, tables: &[&str], key: &str, value: &Value) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let ends_with_newline = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let new_line = format!("{key} = {}", value.to_string().trim());
    let header = tables.join(".");
    let range = if tables.is_empty() {
        Some((0, lines.iter().position(|l| header_name(l).is_some()).unwrap_or(lines.len())))
    } else {
        lines.iter().position(|l| header_name(l) == Some(header.as_str())).map(|h| {
            let end = lines[h + 1..].iter().position(|l| header_name(l).is_some()).map_or(lines.len(), |i| h + 1 + i);
            (h + 1, end)
        })
    };
    match range {
        Some((start, end)) => {
            let at = match lines[start..end].iter().position(|l| is_commented_key(l, key)) {
                Some(i) => start + i + 1,
                None => lines[start..end].iter().rposition(|l| !l.trim().is_empty()).map_or(start, |i| start + i + 1),
            };
            lines.insert(at, new_line);
        }
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[{header}]"));
            lines.push(new_line);
        }
    }
    let mut out = lines.join(newline);
    if ends_with_newline || range.is_none() {
        out.push_str(newline);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{DEFAULT_CONFIG, ParseOutcome, parse};

    fn reparse(text: &str) -> crate::settings::Settings {
        match parse(text) {
            ParseOutcome::Ok { settings, issues } => {
                assert!(issues.is_empty(), "{issues:?}");
                settings
            }
            ParseOutcome::Unreadable(i) => panic!("{i:?}\n{text}"),
        }
    }

    #[test]
    fn replaces_value_keeps_comments() {
        let out = set_value(DEFAULT_CONFIG, &["window", "background_opacity"], EditValue::Float(1.0)).unwrap();
        assert_eq!(reparse(&out).window.background_opacity, 1.0);
        for line in DEFAULT_CONFIG.lines().filter(|l| l.trim_start().starts_with('#')) {
            assert!(out.lines().any(|o| o == line), "消えた: {line}");
        }
        assert!(out.contains("# 0.0〜1.0。1.0 で透かさない"));
    }

    #[test]
    fn refuses_unreadable() {
        assert_eq!(set_value("theme = ", &["theme"], EditValue::Str("oled".into())), Err(EditError::Unreadable));
    }

    #[test]
    fn commented_key_gets_line_after_comment() {
        let out = set_value(DEFAULT_CONFIG, &["accent"], EditValue::Str("#ff0080".into())).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        let at = lines.iter().position(|l| l.starts_with("# accent = ")).unwrap();
        assert_eq!(lines[at + 1], "accent = \"#ff0080\"");
        assert_eq!(reparse(&out).accent, Some(0xff0080));
    }

    #[test]
    fn commented_key_in_table_gets_line_after_comment() {
        let out = set_value(DEFAULT_CONFIG, &["font", "line_height"], EditValue::Int(22)).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        let at = lines.iter().position(|l| l.starts_with("# line_height = ")).unwrap();
        assert_eq!(lines[at + 1], "line_height = 22");
        assert_eq!(reparse(&out).font.line_height, 22.0);
    }

    #[test]
    fn missing_key_is_added_to_its_table() {
        let out = set_value("[window]\nblur = true\n\n[font]\nsize = 14\n", &["window", "background_opacity"], EditValue::Float(0.5)).unwrap();
        assert_eq!(reparse(&out).window.background_opacity, 0.5);
        assert!(out.starts_with("[window]\nblur = true\nbackground_opacity = 0.5\n"), "{out}");
    }

    #[test]
    fn missing_table_is_created() {
        let out = set_value("theme = \"fude\"\n", &["tree", "show_heavy_dirs"], EditValue::Bool(true)).unwrap();
        assert!(reparse(&out).tree.show_heavy_dirs);
    }

    #[test]
    fn nested_table_is_created() {
        let out = set_value("theme = \"fude\"\n", &["keybindings", "windows", "close"], EditValue::StrList(vec!["ctrl-q".into()])).unwrap();
        assert_eq!(reparse(&out).keybindings.windows, vec![("close".to_string(), vec!["ctrl-q".to_string()])]);
    }

    #[test]
    fn default_value_is_still_written() {
        let out = set_value(DEFAULT_CONFIG, &["theme"], EditValue::Str("fude".into())).unwrap();
        assert!(out.lines().any(|l| l == "theme = \"fude\""));
    }

    #[test]
    fn remove_value_drops_line_and_keeps_comments() {
        let text = "# a\naccent = \"#ff0080\"  # c\ntheme = \"fude\"\n";
        assert_eq!(remove_value(text, &["accent"]).unwrap(), "# a\ntheme = \"fude\"\n");
        let crlf = text.replace('\n', "\r\n");
        assert_eq!(remove_value(&crlf, &["accent"]).unwrap(), "# a\r\ntheme = \"fude\"\r\n");
        assert_eq!(remove_value(text, &["font", "line_height"]).unwrap(), text);
        let nested = "[font]\n# line_height = 18\nline_height = 22\nsize = 14\n";
        assert_eq!(remove_value(nested, &["font", "line_height"]).unwrap(), "[font]\n# line_height = 18\nsize = 14\n");
        let inline = "font = { size = 14, line_height = 22 }\n";
        assert!(!remove_value(inline, &["font", "line_height"]).unwrap().contains("line_height"));
        assert_eq!(remove_value("theme = ", &["theme"]), Err(EditError::Unreadable));
    }

    #[test]
    fn crlf_is_preserved() {
        let crlf = DEFAULT_CONFIG.replace('\n', "\r\n");
        for (path, value) in [
            (&["window", "blur"][..], EditValue::Bool(false)),
            (&["accent"][..], EditValue::Str("#ff0080".into())),
            (&["tree", "heavy_dirs"][..], EditValue::StrList(vec!["a".into()])),
        ] {
            let out = set_value(&crlf, path, value).unwrap();
            assert!(!out.replace("\r\n", "").contains('\n'), "単独の LF がある: {path:?}");
        }
    }

    #[test]
    fn inline_table_gets_key_without_duplicate_table() {
        let out = set_value("window = { blur = false }\n", &["window", "background_opacity"], EditValue::Float(1.0)).unwrap();
        let s = reparse(&out);
        assert_eq!(s.window.background_opacity, 1.0);
        assert!(!s.window.blur);
    }

    #[test]
    fn dotted_keys_get_key_without_duplicate_table() {
        let out = set_value("window.blur = false\n", &["window", "background_opacity"], EditValue::Float(0.5)).unwrap();
        let s = reparse(&out);
        assert_eq!(s.window.background_opacity, 0.5);
        assert!(!s.window.blur);
    }

    #[test]
    fn quoted_header_is_found() {
        let out = set_value("[ \"window\" ]\nblur = false\n", &["window", "background_opacity"], EditValue::Float(0.5)).unwrap();
        let s = reparse(&out);
        assert_eq!(s.window.background_opacity, 0.5);
        assert!(!s.window.blur);
    }

    #[test]
    fn inline_comment_after_value_is_kept() {
        let out = set_value(DEFAULT_CONFIG, &["cursor", "shape"], EditValue::Str("bar".into())).unwrap();
        assert!(out.lines().any(|l| l.starts_with("shape = \"bar\"") && l.contains("# \"block\" / \"bar\" / \"underline\"")), "{out}");
    }
}
