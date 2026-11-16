//! `fude hook`: Claude Code の hook の入力を縮めた信号にし、起動中の fude に送る（設計書 §5.8）。
//! GUI は起動しない。どんな失敗でも終了コード 0 で終わる（Claude Code にエラーを出させないため）。

use std::ffi::OsString;
use std::io::Read;
use std::time::Duration;

use fude_core::claude::{HookCommand, HookSignal, hook_config, signal_from_hook};
pub use fude_core::claude_settings::HookAdmin;
use fude_ipc::Message;

/// 標準入力を読む上限。超えた分は読まない（読めずに捨てる）。
pub const HOOK_STDIN_MAX: u64 = 8 * 1024 * 1024;
/// 起動中の fude の応答を待つ時間。
pub const HOOK_SEND_TIMEOUT: Duration = Duration::from_secs(1);

const USAGE: &str = "使い方: fude hook [--print-config | --signal] [--terminal <id>] [--instance <名前>] | fude hook --install | --uninstall | --status";

#[derive(Debug, PartialEq)]
pub enum HookArgs {
    PrintConfig,
    /// settings.json への登録・解除・状態（9b の設計書 §5.6）。
    Admin(HookAdmin),
    Forward { signal_input: bool, terminal: Option<String>, instance: Option<String> },
}

/// `hook` を除いた引数。
pub fn parse_hook_args(args: &[OsString]) -> Result<HookArgs, String> {
    let strings: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    if let Some(admin) = fude_core::claude_settings::parse_admin(&strings) {
        return admin.map(HookArgs::Admin);
    }
    let mut signal_input = false;
    let mut terminal = None;
    let mut instance = None;
    let mut iter = args.iter().map(|a| a.to_string_lossy().into_owned());
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--print-config" => return Ok(HookArgs::PrintConfig),
            "--signal" => signal_input = true,
            "--terminal" => terminal = Some(iter.next().ok_or(USAGE)?),
            "--instance" => instance = Some(iter.next().ok_or(USAGE)?),
            _ => return Err(USAGE.to_string()),
        }
    }
    Ok(HookArgs::Forward { signal_input, terminal, instance })
}

/// 標準入力の中身から送る Message。signal_input なら HookSignal の JSON、そうでなければ Claude Code の入力。
/// terminal が None か、関係の無いイベントなら None。
pub fn hook_message(input: &[u8], signal_input: bool, terminal: Option<String>) -> Option<Message> {
    let terminal = terminal.filter(|t| !t.is_empty())?;
    let signal: HookSignal = if signal_input {
        serde_json::from_slice(input).ok()?
    } else {
        signal_from_hook(&serde_json::from_slice(input).ok()?)?
    };
    Some(Message::Hook { terminal, signal })
}

/// instance（無ければ環境の FUDE_INSTANCE）で決まるパイプの名前。
pub fn hook_endpoint(instance: Option<&str>, env: &dyn Fn(&str) -> Option<String>) -> String {
    match instance {
        Some(name) => {
            let name = name.to_string();
            fude_ipc::endpoint_name(&|k| if k == "FUDE_INSTANCE" { Some(name.clone()) } else { env(k) })
        }
        None => fude_ipc::endpoint_name(env),
    }
}

/// hook に登録するこの exe のパス。Windows はパスの \ を / にする（Git Bash でも PowerShell でも読めるように）。
pub fn own_exe_for_hook() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(fude_core::paths::simplify_verbatim(exe).to_string_lossy().replace('\\', "/"))
}

/// settings.json への登録・解除・状態を実行し、そのあとの状態を返す。
fn admin_status(admin: &HookAdmin) -> fude_core::claude_settings::HookStatus {
    use fude_core::claude_settings::{home_from_env, settings_path};
    let env = |k: &str| std::env::var(k).ok();
    let path = settings_path(&env, home_from_env(&env));
    let own = own_exe_for_hook().unwrap_or_default();
    let make = |c: Option<String>| HookCommand::Exec(c.unwrap_or_else(|| own.clone()));
    fude_core::claude_settings::run_admin(admin, path.as_deref(), &make)
}

/// settings.json への登録・解除・状態を実行し、そのあとの状態を 1 行の JSON で書く。
fn run_admin(admin: &HookAdmin) {
    let status = admin_status(admin);
    crate::instance::attach_parent_console();
    println!("{}", serde_json::to_string(&status).unwrap_or_default());
}

/// この PC の Claude Code の hook を外し、結果をログの 1 行にする（uninstall-cleanup が使う）。
pub(crate) fn uninstall_this_pc() -> String {
    let status = admin_status(&HookAdmin::Uninstall);
    format!("この PC の hook: {}", serde_json::to_string(&status).unwrap_or_default())
}

/// この exe の hook の設定例。
fn print_config() {
    let Some(path) = own_exe_for_hook() else { return };
    let config = hook_config(&HookCommand::Exec(path));
    crate::instance::attach_parent_console();
    println!("{}", serde_json::to_string_pretty(&config).unwrap_or_default());
}

/// 戻り値は終了コード（使い方の誤りだけ 2、それ以外は 0）。
pub fn hook_main(args: &[OsString]) -> i32 {
    match parse_hook_args(args) {
        Ok(HookArgs::PrintConfig) => print_config(),
        Ok(HookArgs::Admin(admin)) => run_admin(&admin),
        Ok(HookArgs::Forward { signal_input, terminal, instance }) => {
            let mut input = Vec::new();
            if std::io::stdin().lock().take(HOOK_STDIN_MAX).read_to_end(&mut input).is_err() {
                return 0;
            }
            let terminal = terminal.or_else(|| std::env::var("FUDE_TERMINAL_ID").ok());
            if let Some(msg) = hook_message(&input, signal_input, terminal) {
                let name = hook_endpoint(instance.as_deref(), &|k| std::env::var(k).ok());
                let _ = fude_ipc::send(&name, &msg, HOOK_SEND_TIMEOUT);
            }
        }
        Err(usage) => {
            crate::instance::report_error(&usage);
            return 2;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{HOOK_SEND_TIMEOUT, HookAdmin, HookArgs, hook_endpoint, hook_message, parse_hook_args};
    use fude_core::claude::HookSignal;
    use fude_ipc::Message;
    use std::ffi::OsString;

    #[test]
    fn parses_hook_args() {
        let a = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(parse_hook_args(&a(&[])), Ok(HookArgs::Forward { signal_input: false, terminal: None, instance: None }));
        assert_eq!(parse_hook_args(&a(&["--print-config"])), Ok(HookArgs::PrintConfig));
        assert_eq!(
            parse_hook_args(&a(&["--signal", "--terminal", "p3", "--instance", "dev"])),
            Ok(HookArgs::Forward { signal_input: true, terminal: Some("p3".into()), instance: Some("dev".into()) })
        );
        assert!(parse_hook_args(&a(&["--terminal"])).is_err());
        assert!(parse_hook_args(&a(&["--what"])).is_err());
        assert_eq!(parse_hook_args(&a(&["--install"])), Ok(HookArgs::Admin(HookAdmin::Install { command: None })));
        assert_eq!(parse_hook_args(&a(&["--uninstall"])), Ok(HookArgs::Admin(HookAdmin::Uninstall)));
        assert!(parse_hook_args(&a(&["--status", "--x"])).is_err());
    }

    #[test]
    fn no_terminal_sends_nothing() {
        assert_eq!(hook_message(br#"{"hook_event_name":"Stop"}"#, false, None), None);
    }

    #[test]
    fn hook_message_from_big_input() {
        let input = serde_json::json!({"hook_event_name": "Stop", "last_assistant_message": "x".repeat(3_000_000)}).to_string();
        let msg = hook_message(input.as_bytes(), false, Some("p2".into())).unwrap();
        assert_eq!(msg, Message::Hook { terminal: "p2".into(), signal: HookSignal::Waiting });
        assert!(serde_json::to_string(&msg).unwrap().len() < 200);
        assert_eq!(hook_message(b"not json", false, Some("p2".into())), None);
        let signal = serde_json::to_string(&HookSignal::Working).unwrap();
        assert_eq!(
            hook_message(signal.as_bytes(), true, Some("p2".into())),
            Some(Message::Hook { terminal: "p2".into(), signal: HookSignal::Working })
        );
    }

    #[test]
    fn endpoint_uses_instance() {
        let env = |k: &str| match k {
            "USERNAME" => Some("neko".to_string()),
            "FUDE_INSTANCE" => Some("perf".to_string()),
            _ => None,
        };
        assert_eq!(hook_endpoint(Some("dev"), &env), "fude-dev-neko");
        assert_eq!(hook_endpoint(None, &env), "fude-perf-neko");
    }

    #[test]
    fn not_running_returns_quickly() {
        let start = std::time::Instant::now();
        let msg = Message::Hook { terminal: "p1".into(), signal: HookSignal::Waiting };
        assert!(fude_ipc::send("fude-test-hook-nobody", &msg, HOOK_SEND_TIMEOUT).is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
