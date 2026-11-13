//! 起動時の引数から、何をするか（Launch）を決める。

use std::ffi::OsString;
use std::path::Path;

use fude_core::location::FileLocation;
use fude_core::paths::simplify_verbatim;
use fude_core::workspace::{ProjectRoot, ShellKind};

#[derive(Debug, PartialEq)]
pub enum Launch {
    /// 前面に出すだけ（起動していなければ、前回の状態で始める）。
    Activate,
    /// プロジェクトを開く。shell は新しく開くときの既定のシェルの上書き。
    Open { root: ProjectRoot, shell: Option<ShellKind> },
    /// ファイルを開く（ターミナルは起動しない）。root_hint は WSL 側の fude が探したプロジェクトのルート。
    OpenFile { file: FileLocation, root_hint: Option<ProjectRoot> },
}

const USAGE: &str = "使い方: fude [フォルダ | ファイル]";

/// args は実行ファイル名を除いたもの。引数なし → Activate。1 つ → cwd から解決して canonicalize し、
/// simplify_verbatim に通す。フォルダなら Open（ProjectRoot::from_host_path）、ファイルなら OpenFile
/// （FileLocation::from_host_path）。無ければ Err(理由)。
/// `--wsl-file <distro> <root> <file>` → OpenFile（root と file は Linux の絶対パス。WSL 側の fude だけが使う）。
/// `--wsl <distro> <Linux の絶対パス>` → from_wsl_path で作り、シェルは WSL にする。この形は WSL 側の fude だけが
/// 使い、Linux 側で解決済みなので canonicalize しない。それ以外の数の引数は Err(使い方)。
pub fn parse_launch(args: &[OsString], cwd: &Path) -> Result<Launch, String> {
    if args.first().is_some_and(|a| a == "--wsl-file") {
        let [_, distro, root, file] = args else { return Err(USAGE.to_string()) };
        let (Some(distro), Some(root), Some(file)) = (distro.to_str(), root.to_str(), file.to_str()) else {
            return Err("WSL のパスを読めない".to_string());
        };
        if distro.is_empty() || !root.starts_with('/') || !file.starts_with('/') {
            return Err(USAGE.to_string());
        }
        return Ok(Launch::OpenFile {
            file: FileLocation::from_wsl_path(distro, file),
            root_hint: Some(ProjectRoot::from_wsl_path(distro, root)),
        });
    }
    if args.first().is_some_and(|a| a == "--wsl") {
        let [_, distro, path] = args else { return Err(USAGE.to_string()) };
        let (Some(distro), Some(path)) = (distro.to_str(), path.to_str()) else {
            return Err("WSL のパスを読めない".to_string());
        };
        if distro.is_empty() || !path.starts_with('/') {
            return Err(USAGE.to_string());
        }
        let root = ProjectRoot::from_wsl_path(distro, path);
        return Ok(Launch::Open { root, shell: Some(ShellKind::Wsl { distro: distro.to_string() }) });
    }
    let path = match args {
        [] => return Ok(Launch::Activate),
        [path] => cwd.join(path),
        _ => return Err(USAGE.to_string()),
    };
    let canonical = std::fs::canonicalize(&path).map_err(|e| format!("開けない: {}: {e}", path.display()))?;
    let canonical = simplify_verbatim(canonical);
    if canonical.is_dir() {
        Ok(Launch::Open { root: ProjectRoot::from_host_path(canonical), shell: None })
    } else {
        Ok(Launch::OpenFile { file: FileLocation::from_host_path(canonical), root_hint: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fude_core::location::FileLocation;
    use fude_core::workspace::{ProjectRoot, ShellKind};
    use std::ffi::OsString;
    use std::path::Path;

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
        assert_eq!(launch, Launch::Open { root: canonical(dir.path()), shell: None });
        let Launch::Open { root: ProjectRoot::Local(p), .. } = launch else { panic!() };
        assert!(!p.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn relative_path_resolves() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert_eq!(parse_launch(&args(&["sub"]), dir.path()), Ok(Launch::Open { root: canonical(&dir.path().join("sub")), shell: None }));
    }

    #[test]
    fn file_arg_becomes_open_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        let launch = parse_launch(&args(&["a.rs"]), dir.path()).unwrap();
        let Launch::OpenFile { file: FileLocation::Local(p), root_hint: None } = &launch else { panic!("{launch:?}") };
        assert!(p.ends_with("a.rs"));
        assert!(!p.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn missing_is_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(parse_launch(&args(&["missing"]), dir.path()).is_err());
    }

    #[test]
    fn wsl_file_args() {
        let l = parse_launch(&args(&["--wsl-file", "Ubuntu", "/home/x/p", "/home/x/p/a b.rs"]), Path::new(".")).unwrap();
        assert_eq!(
            l,
            Launch::OpenFile {
                file: FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/x/p/a b.rs".into() },
                root_hint: Some(ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x/p".into() }),
            }
        );
        assert!(parse_launch(&args(&["--wsl-file", "Ubuntu", "rel", "/a"]), Path::new(".")).is_err());
        assert!(parse_launch(&args(&["--wsl-file", "Ubuntu", "/a", "rel"]), Path::new(".")).is_err());
        assert!(parse_launch(&args(&["--wsl-file", "Ubuntu", "/a"]), Path::new(".")).is_err());
    }

    fn ubuntu() -> ShellKind {
        ShellKind::Wsl { distro: "Ubuntu".into() }
    }

    #[test]
    fn wsl_launch_args() {
        let launch = parse_launch(&args(&["--wsl", "Ubuntu", "/home/x/my proj"]), Path::new("."));
        let root = ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x/my proj".into() };
        assert_eq!(launch, Ok(Launch::Open { root, shell: Some(ubuntu()) }));
    }

    #[test]
    fn wsl_launch_mnt_is_local() {
        let launch = parse_launch(&args(&["--wsl", "Ubuntu", "/mnt/c/Users/x"]), Path::new("."));
        let root = ProjectRoot::Local(std::path::PathBuf::from(r"C:\Users\x"));
        assert_eq!(launch, Ok(Launch::Open { root, shell: Some(ubuntu()) }));
    }

    #[test]
    fn wsl_launch_needs_abs_path() {
        assert!(parse_launch(&args(&["--wsl", "Ubuntu", "x"]), Path::new(".")).is_err());
        assert!(parse_launch(&args(&["--wsl", "Ubuntu"]), Path::new(".")).is_err());
        assert!(parse_launch(&args(&["--wsl", "Ubuntu", "/a", "/b"]), Path::new(".")).is_err());
    }

    #[test]
    fn extra_args_are_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(parse_launch(&args(&[".", "."]), dir.path()).is_err());
    }
}
