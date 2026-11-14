//! WSL に置く Linux 用の fude。`fude [path]` で Windows の fude にフォルダかファイルを開かせ、`fude serve` で
//! Windows の fude からのファイルの要求を処理する。`fude version` は版の印を出す。

use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
enum Cli {
    Open(PathBuf),
    Serve,
    /// root の変更を監視し、WatchBatch を JSON 1 行ずつ標準出力に書く。標準入力が閉じたら終わる。
    Watch(PathBuf),
    Version,
    /// Claude Code の hook。信号に縮めて Windows の fude.exe に渡す。print_config なら設定例を出す。
    Hook { print_config: bool },
}

/// hook の標準入力を読む上限。
const HOOK_STDIN_MAX: u64 = 8 * 1024 * 1024;
/// fude.exe の hook の終わりを待つ時間。
const HOOK_WAIT: Duration = Duration::from_secs(3);

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
        [cmd] if cmd == "hook" => Ok(Cli::Hook { print_config: false }),
        [cmd, flag] if cmd == "hook" && flag == "--print-config" => Ok(Cli::Hook { print_config: true }),
        [cmd, root] if cmd == "watch" => Ok(Cli::Watch(PathBuf::from(root))),
        [path] => Ok(Cli::Open(PathBuf::from(path))),
        _ => Err("使い方: fude [フォルダ | ファイル] | fude serve | fude watch <フォルダ> | fude hook [--print-config] | fude version".to_string()),
    }
}

/// fude.exe の場所を書いたファイルの候補。自分の隣の host-exe（開発中の試験用の置き場所）、
/// ~/.config/fude/host-exe の順。
fn host_exe_files(own_exe: Option<&Path>, home: Option<&str>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(dir) = own_exe.and_then(Path::parent) {
        files.push(dir.join("host-exe"));
    }
    if let Some(home) = home {
        files.push(Path::new(home).join(".config/fude/host-exe"));
    }
    files
}

/// host-exe と distro と絶対パスから、起動するコマンドと引数を作る。
fn open_command(host_exe: &str, distro: &str, abs: &Path) -> (String, Vec<String>) {
    let args = vec!["--wsl".to_string(), distro.to_string(), abs.to_string_lossy().into_owned()];
    (host_exe.to_string(), args)
}

/// ファイルを開かせるコマンドと引数。root は fude_core::location::find_project_root で探したもの。
fn open_file_command(host_exe: &str, distro: &str, root: &Path, file: &Path) -> (String, Vec<String>) {
    let args = vec![
        "--wsl-file".to_string(),
        distro.to_string(),
        root.to_string_lossy().into_owned(),
        file.to_string_lossy().into_owned(),
    ];
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
        Ok(Cli::Watch(root)) => watch(root),
        Ok(Cli::Hook { print_config: true }) => {
            print_hook_config();
            0
        }
        Ok(Cli::Hook { print_config: false }) => {
            hook();
            0
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

/// root を監視して、変更を 1 行ずつ書く。標準入力が閉じる（Windows 側が wsl.exe を終わらせる）か、
/// 書けなくなったら終わる。
fn watch(root: PathBuf) -> i32 {
    let on_batch = Box::new(|batch: fude_fs::watch::WatchBatch| {
        use std::io::Write;
        let Ok(line) = serde_json::to_string(&batch) else { return };
        let mut out = io::stdout().lock();
        if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
            std::process::exit(0);
        }
    });
    let _watcher = match fude_fs::watch::LocalWatcher::start(root, on_batch) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("fude watch: {e}");
            return 1;
        }
    };
    let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
    0
}

/// fude.exe に渡すコマンドと引数。
fn hook_forward_command(host_exe: &str, terminal: &str, instance: Option<&str>) -> (String, Vec<String>) {
    let mut args = vec!["hook".to_string(), "--signal".to_string(), "--terminal".to_string(), terminal.to_string()];
    if let Some(instance) = instance {
        args.extend(["--instance".to_string(), instance.to_string()]);
    }
    (host_exe.to_string(), args)
}

/// fude.exe の場所（host-exe に書いたもの）。
fn host_exe() -> Option<String> {
    let own_exe = std::env::current_exe().ok();
    let home = std::env::var("HOME").ok();
    host_exe_files(own_exe.as_deref(), home.as_deref())
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
}

fn print_hook_config() {
    let Ok(exe) = std::env::current_exe() else { return };
    let config = fude_core::claude::hook_config(&fude_core::claude::HookCommand::Exec(exe.to_string_lossy().into_owned()));
    println!("{}", serde_json::to_string_pretty(&config).unwrap_or_default());
}

/// 標準入力の hook の入力を信号に縮め、fude.exe hook --signal に渡す。どの失敗でも何もせずに戻る。
fn hook() {
    use std::io::{Read, Write};
    let mut input = Vec::new();
    if io::stdin().lock().take(HOOK_STDIN_MAX).read_to_end(&mut input).is_err() {
        return;
    }
    let Some(signal) = serde_json::from_slice(&input).ok().and_then(|v| fude_core::claude::signal_from_hook(&v)) else { return };
    let Some(terminal) = std::env::var("FUDE_TERMINAL_ID").ok().filter(|t| !t.is_empty()) else { return };
    let Some(host) = host_exe() else { return };
    let Ok(line) = serde_json::to_vec(&signal) else { return };
    let instance = std::env::var("FUDE_INSTANCE").ok().filter(|v| !v.is_empty());
    let (cmd, args) = hook_forward_command(&host, &terminal, instance.as_deref());
    let Ok(mut child) = Command::new(&cmd).args(&args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&line);
    }
    let deadline = Instant::now() + HOOK_WAIT;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            _ => return,
        }
    }
}

/// 子プロセスを新しいセッションで起動する。WSL は、起動した wsl.exe のセッションが終わるときに、その中から
/// 起動した Windows のプロセスも終わらせる。fude が動いていなかったときに起動した fude.exe が、端末を閉じると
/// 一緒に消えないようにする。
#[cfg(unix)]
fn new_session(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    // SAFETY: fork と exec の間で呼ぶのは async-signal-safe な setsid だけである。
    unsafe {
        command.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn new_session(_command: &mut Command) {}

/// path を Windows の fude に開かせ、その終了コードを返す（OPEN_WAIT を過ぎたら 0）。
/// フォルダならプロジェクトとして、ファイルならそのファイルを（プロジェクトのルートを探して添えて）開かせる。
fn open(path: &Path) -> Result<i32, String> {
    let distro = std::env::var("WSL_DISTRO_NAME").ok().filter(|d| !d.is_empty()).ok_or(NOT_IN_WSL)?;
    let host_exe = host_exe().ok_or(NO_HOST_EXE)?;
    let abs = std::fs::canonicalize(path).map_err(|e| format!("開けない: {}: {e}", path.display()))?;
    let (cmd, args) = if abs.is_dir() {
        open_command(&host_exe, &distro, &abs)
    } else {
        let root = fude_core::location::find_project_root(&abs, &|d| d.join(".git").exists());
        open_file_command(&host_exe, &distro, &root, &abs)
    };
    // 標準入出力はつながない。待たずに戻ったあとで fude が書いても、閉じた口に書いて落ちないようにする。
    let mut command = Command::new(&cmd);
    command.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    new_session(&mut command);
    let mut child = command.spawn().map_err(|e| format!("Windows の fude を起動できない: {cmd}: {e}"))?;
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
        assert_eq!(parse(&["watch", "/home/x/p"]), Ok(Cli::Watch(PathBuf::from("/home/x/p"))));
        assert_eq!(parse(&["~/x"]), Ok(Cli::Open(PathBuf::from("~/x"))));
        assert!(parse(&["a", "b"]).is_err());
    }

    #[test]
    fn parse_hook() {
        assert_eq!(parse(&["hook"]), Ok(Cli::Hook { print_config: false }));
        assert_eq!(parse(&["hook", "--print-config"]), Ok(Cli::Hook { print_config: true }));
    }

    #[test]
    fn hook_forward_args() {
        let (cmd, args) = hook_forward_command("/mnt/c/f/fude.exe", "p4", Some("dev"));
        assert_eq!(cmd, "/mnt/c/f/fude.exe");
        assert_eq!(args, vec!["hook", "--signal", "--terminal", "p4", "--instance", "dev"]);
        let (_, args) = hook_forward_command("/mnt/c/f/fude.exe", "p4", None);
        assert_eq!(args, vec!["hook", "--signal", "--terminal", "p4"]);
    }

    #[test]
    fn host_exe_files_prefer_own_dir() {
        let files = host_exe_files(Some(Path::new("/tmp/t/fude")), Some("/home/x"));
        assert_eq!(files, vec![PathBuf::from("/tmp/t/host-exe"), PathBuf::from("/home/x/.config/fude/host-exe")]);
        assert_eq!(host_exe_files(None, Some("/home/x")), vec![PathBuf::from("/home/x/.config/fude/host-exe")]);
    }

    #[test]
    fn open_file_command_args() {
        let (cmd, args) =
            open_file_command("/mnt/c/f/fude.exe", "Ubuntu", Path::new("/home/x/p"), Path::new("/home/x/p/作業 a.md"));
        assert_eq!(cmd, "/mnt/c/f/fude.exe");
        assert_eq!(args, vec!["--wsl-file", "Ubuntu", "/home/x/p", "/home/x/p/作業 a.md"]);
    }

    #[test]
    fn open_args_keep_spaces() {
        let (cmd, args) = open_command("/mnt/c/a b/fude.exe", "Ubuntu", Path::new("/home/x/作業/my proj"));
        assert_eq!(cmd, "/mnt/c/a b/fude.exe");
        assert_eq!(args, vec!["--wsl", "Ubuntu", "/home/x/作業/my proj"]);
    }
}
