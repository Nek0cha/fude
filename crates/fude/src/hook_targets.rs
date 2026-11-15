//! Claude Code の hook の登録先（この PC と WSL のディストリビューション）の状態を調べ、登録・解除する
//! （9b の設計書 `2026-11-15-settings-panel-hook-design.md` §5.4、§5.7）。
//! どの関数も WSL の起動や配置を待つことがあるので、裏のスレッドから呼ぶ。

use fude_core::claude::HookCommand;
use fude_core::claude_settings::{HookAdmin, HookStatus, home_from_env, run_admin, settings_path};

/// 登録先。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum HookTarget {
    /// この PC（Windows か mac）。
    ThisPc,
    /// WSL のディストリビューション（Windows だけ）。
    Wsl(String),
}

/// 登録先の、パネルに出す状態。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TargetState {
    /// まだ調べていない（WSL は押すまで調べない。§5.4）。
    Unchecked,
    /// 調べている、または登録・解除している。
    Checking,
    Known(HookStatus),
    /// WSL に fude が置かれていない（登録するときに置く）。
    NotInstalled,
}

/// 何をするか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HookOp {
    Check,
    Install,
    Uninstall,
}

/// `fude hook --status` などの出力から状態を読む。標準出力の最後の空でない行を JSON として読み、
/// 読めなければ標準エラーの最初の行を理由にする。
pub(crate) fn parse_status_output(stdout: &[u8], stderr: &[u8]) -> HookStatus {
    let out = String::from_utf8_lossy(stdout);
    if let Some(status) = out.lines().rev().find(|l| !l.trim().is_empty()).and_then(|l| serde_json::from_str(l.trim()).ok()) {
        return status;
    }
    let err = String::from_utf8_lossy(stderr);
    let reason = err.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("fude から答えが返りませんでした");
    HookStatus::Unreadable(reason.to_string())
}

/// WSL を呼べなかったときの状態。
pub(crate) fn state_from_io(err: std::io::Error) -> TargetState {
    TargetState::Known(HookStatus::Unreadable(err.to_string()))
}

/// この PC の settings.json に対して op を行う。
fn run_this_pc(op: HookOp) -> TargetState {
    let env = |k: &str| std::env::var(k).ok();
    let path = settings_path(&env, home_from_env(&env));
    let own = crate::hook_cli::own_exe_for_hook().unwrap_or_default();
    let make = |c: Option<String>| HookCommand::Exec(c.unwrap_or_else(|| own.clone()));
    let admin = match op {
        HookOp::Check => HookAdmin::Status { command: None },
        HookOp::Install => HookAdmin::Install { command: None },
        HookOp::Uninstall => HookAdmin::Uninstall,
    };
    TargetState::Known(run_admin(&admin, path.as_deref(), &make))
}

/// WSL の distro に対して op を行う（設計書 §5.4、§5.7）。
/// fude が無ければ、登録のときだけ置く。古ければ、どの op でも置き直してから聞く。
#[cfg(windows)]
fn run_wsl(distro: &str, bin: &str, op: HookOp) -> TargetState {
    use fude_fs::wsl::{RUN_NOT_INSTALLED, STATUS_TIMEOUT, deploy, installed_version, run_fude};
    let installed = match installed_version(distro, bin) {
        Ok(v) => v,
        Err(e) => return state_from_io(e),
    };
    if installed.is_none() && op != HookOp::Install {
        return TargetState::NotInstalled;
    }
    let Some((bytes, bundled)) = crate::wsl_host::bundled_linux() else {
        return TargetState::Known(HookStatus::Unreadable("Linux 用の fude が同梱されていません".to_string()));
    };
    if crate::wsl_host::needs_deploy(installed.as_deref(), bundled) {
        let host_exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => return state_from_io(e),
        };
        if let Err(e) = deploy(distro, bin, bytes, &host_exe) {
            return state_from_io(e);
        }
    }
    let args: &[&str] = match op {
        HookOp::Check => &["hook", "--status", "--command", bin],
        HookOp::Install => &["hook", "--install", "--command", bin],
        HookOp::Uninstall => &["hook", "--uninstall"],
    };
    match run_fude(distro, bin, args, STATUS_TIMEOUT) {
        Ok(out) if out.status.code() == Some(RUN_NOT_INSTALLED) => TargetState::NotInstalled,
        Ok(out) => TargetState::Known(parse_status_output(&out.stdout, &out.stderr)),
        Err(e) => state_from_io(e),
    }
}

/// target に対して op を行い、そのあとの状態を返す。wsl_bin は WSL 側の fude の置き場所（WslHosts::bin）。
pub(crate) fn run(target: &HookTarget, op: HookOp, wsl_bin: &str) -> TargetState {
    match target {
        HookTarget::ThisPc => run_this_pc(op),
        #[cfg(windows)]
        HookTarget::Wsl(distro) => run_wsl(distro, wsl_bin, op),
        #[cfg(not(windows))]
        HookTarget::Wsl(_) => {
            let _ = (op, wsl_bin);
            TargetState::Known(HookStatus::Unreadable("WSL は Windows だけで使えます".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fude_core::claude_settings::HookStatus;

    #[test]
    fn status_output_parses() {
        assert_eq!(parse_status_output(b"{\"status\":\"registered\"}\n", b""), HookStatus::Registered);
        assert_eq!(
            parse_status_output(b"noise\n{\"status\":\"unreadable\",\"reason\":\"x\"}\n", b""),
            HookStatus::Unreadable("x".into())
        );
    }

    #[test]
    fn garbage_output_is_unreadable() {
        assert_eq!(parse_status_output(b"", "使い方: fude ...\nmore\n".as_bytes()), HookStatus::Unreadable("使い方: fude ...".into()));
        assert!(matches!(parse_status_output(b"", b""), HookStatus::Unreadable(r) if !r.is_empty()));
    }

    #[test]
    fn status_timeout_becomes_unreadable() {
        let err = std::io::Error::new(std::io::ErrorKind::TimedOut, "wsl.exe が 10 秒以内に終わらない");
        match state_from_io(err) {
            TargetState::Known(HookStatus::Unreadable(r)) => assert!(r.contains("秒以内に終わらない"), "{r}"),
            other => panic!("{other:?}"),
        }
    }
}
