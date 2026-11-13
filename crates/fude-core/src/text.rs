//! テキストファイルの読み書きの規則。UTF-8（BOM の有無を保つ）だけを扱い、改行は開いたときの形で保存する。

/// 開けるテキストの大きさの上限。
pub const MAX_TEXT_BYTES: u64 = 8 * 1024 * 1024;
/// バイナリかを見る先頭の大きさ。
const SNIFF: usize = 8 * 1024;
const BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

/// 開いたテキスト。text の改行は常に `\n`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDoc {
    pub text: String,
    pub line_ending: LineEnding,
    pub bom: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// 先頭 8KiB に NUL がある。
    Binary,
    NotUtf8,
    /// 大きさ（バイト）。
    TooLarge(u64),
}

/// バイト列をテキストにする。CRLF が LF 単独より多ければ CrLf（混在は多い方に揃える）。
pub fn decode_text(bytes: &[u8]) -> Result<TextDoc, OpenError> {
    let len = bytes.len() as u64;
    if len > MAX_TEXT_BYTES {
        return Err(OpenError::TooLarge(len));
    }
    if bytes[..bytes.len().min(SNIFF)].contains(&0) {
        return Err(OpenError::Binary);
    }
    let (bom, body) = match bytes.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    let text = std::str::from_utf8(body).map_err(|_| OpenError::NotUtf8)?;
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    let line_ending = if crlf > lf { LineEnding::CrLf } else { LineEnding::Lf };
    Ok(TextDoc { text: text.replace("\r\n", "\n"), line_ending, bom })
}

/// text（改行は `\n`）を、doc の改行と BOM で書くバイト列にする。
pub fn encode_text(doc: &TextDoc, text: &str) -> Vec<u8> {
    let body = match doc.line_ending {
        LineEnding::Lf => text.to_string(),
        LineEnding::CrLf => text.replace("\r\n", "\n").replace('\n', "\r\n"),
    };
    let mut out = Vec::with_capacity(body.len() + 3);
    if doc.bom {
        out.extend_from_slice(BOM);
    }
    out.extend_from_slice(body.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip_keeps_crlf_bom_and_tail() {
        for raw in [&b"a\r\nb\r\n"[..], b"\xef\xbb\xbfa\nb", b"", b"a\r\nb", "日本語\n".as_bytes()] {
            let doc = decode_text(raw).unwrap();
            assert_eq!(encode_text(&doc, &doc.text), raw, "{raw:?}");
        }
        let doc = decode_text(b"a\r\nb\r\n").unwrap();
        assert_eq!(doc.text, "a\nb\n");
        assert_eq!(doc.line_ending, LineEnding::CrLf);
        assert_eq!(encode_text(&doc, "a\nb\nc\n"), b"a\r\nb\r\nc\r\n");
        let bom = decode_text(b"\xef\xbb\xbfx").unwrap();
        assert!(bom.bom);
        assert_eq!(bom.text, "x");
    }

    #[test]
    fn mixed_line_endings_follow_majority() {
        assert_eq!(decode_text(b"a\r\nb\r\nc\n").unwrap().line_ending, LineEnding::CrLf);
        assert_eq!(decode_text(b"a\nb\nc\r\n").unwrap().line_ending, LineEnding::Lf);
        assert_eq!(decode_text(b"abc").unwrap().line_ending, LineEnding::Lf);
        assert_eq!(decode_text(b"a\r\nb\nc").unwrap().text, "a\nb\nc");
    }

    #[test]
    fn rejects_binary_non_utf8_and_huge() {
        assert_eq!(decode_text(b"a\0b"), Err(OpenError::Binary));
        assert_eq!(decode_text(b"\xff\xfe"), Err(OpenError::NotUtf8));
        assert!(matches!(decode_text(&vec![b'a'; 8 * 1024 * 1024 + 1]), Err(OpenError::TooLarge(_))));
        assert!(decode_text(&vec![b'a'; 8 * 1024 * 1024]).is_ok());
    }

    #[test]
    fn nul_after_first_8k_is_not_binary_check() {
        let mut data = vec![b'a'; 9000];
        data[8500] = 0;
        // 先頭 8KiB だけを見る。NUL は UTF-8 としては正しい。
        assert!(decode_text(&data).is_ok());
    }
}
