//! macOS の Finder の「このアプリケーションで開く」で渡される URL を、起動時の引数と同じ形に直す（10d §5）。

use std::ffi::OsString;
use std::path::PathBuf;

/// `file:///...` か `file://localhost/...` の URL を、%デコードしたパスにする。それ以外の URL、壊れた %、
/// UTF-8 にならないものは None。
pub fn path_from_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

/// 古い macOS が Finder からの起動で付ける `-psn_0_12345` の引数を除く（残すと使い方の失敗で起動しない）。
pub fn strip_psn(args: Vec<OsString>) -> Vec<OsString> {
    args.into_iter().filter(|a| !a.to_string_lossy().starts_with("-psn_")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_file_url() {
        assert_eq!(path_from_url("file:///Users/a/proj/"), Some(PathBuf::from("/Users/a/proj/")));
    }

    #[test]
    fn percent_encoded() {
        assert_eq!(
            path_from_url("file:///Users/a/My%20Proj/%E4%BD%9C%E6%A5%AD.md"),
            Some(PathBuf::from("/Users/a/My Proj/作業.md"))
        );
    }

    #[test]
    fn localhost_host() {
        assert_eq!(path_from_url("file://localhost/tmp/x"), Some(PathBuf::from("/tmp/x")));
    }

    #[test]
    fn rejects_other_urls() {
        assert_eq!(path_from_url("http://example.com/x"), None);
        assert_eq!(path_from_url("file://server/share/x"), None);
        assert_eq!(path_from_url("file:///bad%ZZ"), None);
        assert_eq!(path_from_url("file:///bad%2"), None);
        // UTF-8 でない。
        assert_eq!(path_from_url("file:///bad%FF"), None);
    }

    #[test]
    fn strips_psn() {
        let args = vec![OsString::from("-psn_0_12345"), OsString::from("proj")];
        assert_eq!(strip_psn(args), vec![OsString::from("proj")]);
    }
}
