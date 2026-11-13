//! パスの表記の整え。

use std::path::PathBuf;

/// Windows の canonicalize が付ける `\\?\` を外す。`\\?\C:\x` は `C:\x`、`\\?\UNC\s\x` は `\\s\x` にする。
/// それ以外（ボリューム GUID など）はそのまま返す。シェルの作業ディレクトリや表示名に使うため。
pub fn simplify_verbatim(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else { return path };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return PathBuf::from(rest);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn strips_verbatim_drive_prefix() {
        assert_eq!(simplify_verbatim(PathBuf::from(r"\\?\C:\w\a")), Path::new(r"C:\w\a"));
        assert_eq!(simplify_verbatim(PathBuf::from(r"\\?\C:\")), Path::new(r"C:\"));
    }

    #[test]
    fn verbatim_unc_becomes_plain_unc() {
        assert_eq!(simplify_verbatim(PathBuf::from(r"\\?\UNC\server\share\x")), Path::new(r"\\server\share\x"));
    }

    #[test]
    fn other_paths_are_unchanged() {
        assert_eq!(simplify_verbatim(PathBuf::from("/w/a")), Path::new("/w/a"));
        assert_eq!(simplify_verbatim(PathBuf::from(r"\\?\Volume{x}\a")), Path::new(r"\\?\Volume{x}\a"));
    }
}
