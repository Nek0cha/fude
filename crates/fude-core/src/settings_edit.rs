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
    Ok(format!("{bom}{}", insert_line(body, tables, key, &value.to_value())))
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
    Some(inner.trim())
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
    fn inline_comment_after_value_is_kept() {
        let out = set_value(DEFAULT_CONFIG, &["cursor", "shape"], EditValue::Str("bar".into())).unwrap();
        assert!(out.lines().any(|l| l.starts_with("shape = \"bar\"") && l.contains("# \"block\" / \"bar\" / \"underline\"")), "{out}");
    }
}
