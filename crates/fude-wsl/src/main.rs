//! WSL に置く Linux 用の fude。`fude [path]` で Windows の fude にフォルダを開かせ、`fude serve` で
//! Windows の fude からのファイルの要求を処理する。`fude version` は版の印を出す。

use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
enum Cli {
    Open(PathBuf),
    Serve,
    Version,
}

const BUILD_ID: &str = env!("FUDE_BUILD_ID");

/// Windows の fude の終わりを待つ時間。起動中の fude に渡すだけならすぐ終わる。待ち受けを始めた
/// （fude が動いていなかった）ときは終わらないため、待たずに戻る。
const OPEN_WAIT: Duration = Duration::from_secs(5);

const NOT_IN_WSL: &str = "WSL の中で実行してください";
const NO_HOST_EXE: &str =
    "Windows の fude を一度起動して、WSL のプロジェクトを開いてください（fude がこのコマンドを配置します）";

fn parse_cli(args: &[String]) -> Result<Cli, String> {
    match args {
        [] => Ok(Cli::Open(PathBuf::from("."))),
        [cmd] if cmd == "serve" => Ok(Cli::Serve),
        [cmd] if cmd == "version" => Ok(Cli::Version),
        [path] => Ok(Cli::Open(PathBuf::from(path))),
        _ => Err("使い方: fude [フォルダ] | fude serve | fude version".to_string()),
    }
}

/// host-exe と distro と絶対パスから、起動するコマンドと引数を作る。
fn open_command(host_exe: &str, distro: &str, abs: &Path) -> (String, Vec<String>) {
    let args = vec!["--wsl".to_string(), distro.to_string(), abs.to_string_lossy().into_owned()];
    (host_exe.to_string(), args)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match parse_cli(&args) {
        Ok(Cli::Version) => {
            println!("{BUILD_ID}");
            0
        }
        Ok(Cli::Serve) => {
            let stdin = io::stdin().lock();
            let stdout = BufWriter::new(io::stdout().lock());
            match fude_fs::serve::serve(stdin, stdout, BUILD_ID) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("fude serve: {e}");
                    1
                }
            }
        }
        Ok(Cli::Open(path)) => open(&path).unwrap_or_else(|message| {
            eprintln!("fude: {message}");
            2
        }),
        Err(usage) => {
            eprintln!("{usage}");
            2
        }
    };
    std::process::exit(code);
}

/// path を Windows の fude に開かせ、その終了コードを返す（OPEN_WAIT を過ぎたら 0）。
fn open(path: &Path) -> Result<i32, String> {
    let distro = std::env::var("WSL_DISTRO_NAME").ok().filter(|d| !d.is_empty()).ok_or(NOT_IN_WSL)?;
    let home = std::env::var("HOME").map_err(|_| "HOME が無い".to_string())?;
    let host_exe = std::fs::read_to_string(Path::new(&home).join(".config/fude/host-exe"))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or(NO_HOST_EXE)?;
    let abs = std::fs::canonicalize(path).map_err(|e| format!("フォルダを開けない: {}: {e}", path.display()))?;
    if !abs.is_dir() {
        return Err(format!("フォルダではない: {}", path.display()));
    }
    let (cmd, args) = open_command(&host_exe, &distro, &abs);
    // 標準入出力はつながない。待たずに戻ったあとで fude が書いても、閉じた口に書いて落ちないようにする。
    let mut child = Command::new(&cmd)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Windows の fude を起動できない: {cmd}: {e}"))?;
    let deadline = Instant::now() + OPEN_WAIT;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(0),
            Ok(Some(status)) => return Err(format!("Windows の fude が失敗した（{status}）")),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, String> {
        parse_cli(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parse_cli_rules() {
        assert_eq!(parse(&[]), Ok(Cli::Open(PathBuf::from("."))));
        assert_eq!(parse(&["serve"]), Ok(Cli::Serve));
        assert_eq!(parse(&["version"]), Ok(Cli::Version));
        assert_eq!(parse(&["~/x"]), Ok(Cli::Open(PathBuf::from("~/x"))));
        assert!(parse(&["a", "b"]).is_err());
    }

    #[test]
    fn open_args_keep_spaces() {
        let (cmd, args) = open_command("/mnt/c/a b/fude.exe", "Ubuntu", Path::new("/home/x/作業/my proj"));
        assert_eq!(cmd, "/mnt/c/a b/fude.exe");
        assert_eq!(args, vec!["--wsl", "Ubuntu", "/home/x/作業/my proj"]);
    }
}
