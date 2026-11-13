//! 起動時の引数から、何をするか（Launch）を決める。

use std::ffi::OsString;
use std::path::Path;

use fude_core::paths::simplify_verbatim;
use fude_core::workspace::ProjectRoot;

#[derive(Debug, PartialEq)]
pub enum Launch {
    /// 前面に出すだけ（起動していなければ、前回の状態で始める）。
    Activate,
    /// プロジェクトを開く。
    Open(ProjectRoot),
}

/// args は実行ファイル名を除いたもの。引数なし → Activate。1 つ → cwd から解決して canonicalize し、
/// simplify_verbatim と ProjectRoot::from_host_path に通す。ディレクトリでなければ Err(理由)。2 つ以上は Err(使い方)。
pub fn parse_launch(args: &[OsString], cwd: &Path) -> Result<Launch, String> {
    let path = match args {
        [] => return Ok(Launch::Activate),
        [path] => cwd.join(path),
        _ => return Err("使い方: fude [フォルダ]".to_string()),
    };
    let canonical = std::fs::canonicalize(&path).map_err(|e| format!("フォルダを開けない: {}: {e}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("フォルダではない: {}", path.display()));
    }
    Ok(Launch::Open(ProjectRoot::from_host_path(simplify_verbatim(canonical))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fude_core::workspace::ProjectRoot;
    use std::ffi::OsString;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn canonical(p: &std::path::Path) -> ProjectRoot {
        ProjectRoot::Local(fude_core::paths::simplify_verbatim(std::fs::canonicalize(p).unwrap()))
    }

    #[test]
    fn no_args_activates() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(parse_launch(&[], dir.path()), Ok(Launch::Activate));
    }

    #[test]
    fn dot_resolves_to_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let launch = parse_launch(&args(&["."]), dir.path()).unwrap();
        assert_eq!(launch, Launch::Open(canonical(dir.path())));
        let Launch::Open(ProjectRoot::Local(p)) = launch else { panic!() };
        assert!(!p.to_string_lossy().starts_with(r"\?\"));
    }

    #[test]
    fn relative_path_resolves() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert_eq!(parse_launch(&args(&["sub"]), dir.path()), Ok(Launch::Open(canonical(&dir.path().join("sub")))));
    }

    #[test]
    fn file_or_missing_is_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        assert!(parse_launch(&args(&["f.txt"]), dir.path()).is_err());
        assert!(parse_launch(&args(&["missing"]), dir.path()).is_err());
    }

    #[test]
    fn extra_args_are_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(parse_launch(&args(&[".", "."]), dir.path()).is_err());
    }
}
