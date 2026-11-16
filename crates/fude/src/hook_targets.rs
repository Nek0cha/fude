//! Claude Code の hook の登録先（この PC と WSL のディストリビューション）の状態を調べ、登録・解除する
//! （9b の設計書 `2026-11-15-settings-panel-hook-design.md` §5.4、§5.7）。
//! どの関数も WSL の起動や配置を待つことがあるので、裏のスレッドから呼ぶ。

use fude_core::claude::HookCommand;
use gpui_kit::AppContext as _;
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
        crate::wsl_host::remember_deploy(distro, bin);
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

/// 登録先の状態の文と、押せるボタン（何をするか、文言）。
pub(crate) fn target_row(state: &TargetState) -> (String, Vec<(HookOp, &'static str)>) {
    match state {
        TargetState::Unchecked => ("まだ調べていません".into(), vec![(HookOp::Check, "調べる")]),
        TargetState::Checking => ("調べています…".into(), vec![]),
        TargetState::Known(HookStatus::Registered) => {
            ("登録済み".into(), vec![(HookOp::Install, "登録し直す"), (HookOp::Uninstall, "外す")])
        }
        TargetState::Known(HookStatus::NotRegistered) => ("まだ登録していません".into(), vec![(HookOp::Install, "登録する")]),
        TargetState::Known(HookStatus::Outdated) => {
            ("登録し直してください".into(), vec![(HookOp::Install, "登録し直す"), (HookOp::Uninstall, "外す")])
        }
        TargetState::Known(HookStatus::Unreadable(r)) => (format!("読めません: {r}"), vec![(HookOp::Check, "調べ直す")]),
        TargetState::NotInstalled => {
            ("まだ登録していません（登録するときに fude を WSL に置きます）".into(), vec![(HookOp::Install, "登録する")])
        }
    }
}

/// パネルを開き直したときに調べ直す登録先（9b の設計書 §5.4）。この PC と、一度調べた WSL。
/// まだ調べていない WSL は起こさないため、調べている最中のものは二重に走らせないため除く。
pub(crate) fn rows_to_recheck(targets: &[(HookTarget, TargetState)]) -> Vec<HookTarget> {
    targets
        .iter()
        .filter(|(t, s)| match s {
            TargetState::Checking => false,
            TargetState::Unchecked => *t == HookTarget::ThisPc,
            TargetState::Known(_) | TargetState::NotInstalled => true,
        })
        .map(|(t, _)| t.clone())
        .collect()
}

/// 登録先の名前。
pub(crate) fn target_label(target: &HookTarget) -> String {
    match target {
        HookTarget::ThisPc if cfg!(target_os = "macos") => "この Mac".to_string(),
        HookTarget::ThisPc => "Windows（この PC）".to_string(),
        HookTarget::Wsl(d) => format!("WSL: {d}"),
    }
}

/// target の行を「調べている」にする。調べている最中か、行が無ければ false（走らせない）。
fn begin(targets: &mut [(HookTarget, TargetState)], target: &HookTarget) -> bool {
    match targets.iter_mut().find(|(t, _)| t == target) {
        Some(slot) if slot.1 != TargetState::Checking => {
            slot.1 = TargetState::Checking;
            true
        }
        _ => false,
    }
}

/// hook の登録先の一覧と状態（アプリに 1 つ）。設定パネルとチュートリアルが同じものを見る（9c §5.4）。
pub(crate) struct HookBoard {
    targets: Vec<(HookTarget, TargetState)>,
    /// WSL 側の fude の置き場所。
    wsl_bin: String,
}

impl HookBoard {
    /// この PC を調べ、WSL の一覧を読む。
    pub fn new(cx: &mut gpui_kit::Context<Self>) -> Self {
        #[cfg(windows)]
        let wsl_bin = fude_fs::wsl::default_bin(&|k| std::env::var(k).ok());
        #[cfg(not(windows))]
        let wsl_bin = String::new();
        let mut board = HookBoard { targets: vec![(HookTarget::ThisPc, TargetState::Unchecked)], wsl_bin };
        board.run(HookTarget::ThisPc, HookOp::Check, cx);
        board.reload_distros(cx);
        board
    }

    pub fn targets(&self) -> &[(HookTarget, TargetState)] {
        &self.targets
    }

    /// target に op を裏で行い、終わったら状態を差し替える。調べている最中なら何もしない。
    pub fn run(&mut self, target: HookTarget, op: HookOp, cx: &mut gpui_kit::Context<Self>) {
        if !begin(&mut self.targets, &target) {
            return;
        }
        cx.notify();
        let bin = self.wsl_bin.clone();
        let job_target = target.clone();
        let task = cx.background_spawn(async move { run(&job_target, op, &bin) });
        cx.spawn(async move |this, cx| {
            let state = task.await;
            this.update(cx, |this, cx| {
                if let Some(slot) = this.targets.iter_mut().find(|(t, _)| *t == target) {
                    slot.1 = state;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// WSL のディストリビューションの一覧を読み直す（Windows だけ）。調べた状態は、同じ名前なら残す。
    pub fn reload_distros(&mut self, cx: &mut gpui_kit::Context<Self>) {
        if !cfg!(windows) {
            return;
        }
        let task = cx.background_spawn(async { fude_term::list_distros() });
        cx.spawn(async move |this, cx| {
            let Ok(distros) = task.await else { return };
            this.update(cx, |this, cx| {
                let old = std::mem::take(&mut this.targets);
                let state_of = |t: &HookTarget| old.iter().find(|(o, _)| o == t).map(|(_, s)| s.clone());
                let mut targets = vec![(HookTarget::ThisPc, state_of(&HookTarget::ThisPc).unwrap_or(TargetState::Unchecked))];
                for d in distros {
                    let t = HookTarget::Wsl(d);
                    let s = state_of(&t).unwrap_or(TargetState::Unchecked);
                    targets.push((t, s));
                }
                this.targets = targets;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 開き直したときに調べ直す（rows_to_recheck）。
    pub fn recheck(&mut self, cx: &mut gpui_kit::Context<Self>) {
        for target in rows_to_recheck(&self.targets) {
            self.run(target, HookOp::Check, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fude_core::claude_settings::HookStatus;

    #[test]
    fn board_skips_duplicate_run() {
        let mut targets = vec![(HookTarget::ThisPc, TargetState::Checking), (HookTarget::Wsl("A".into()), TargetState::Unchecked)];
        assert!(!begin(&mut targets, &HookTarget::ThisPc), "調べている最中は二重に走らせない");
        assert_eq!(targets[0].1, TargetState::Checking);
        assert!(begin(&mut targets, &HookTarget::Wsl("A".into())));
        assert_eq!(targets[1].1, TargetState::Checking);
        assert!(!begin(&mut targets, &HookTarget::Wsl("B".into())), "無い登録先は何もしない");
    }

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

    #[test]
    fn row_texts_for_each_state() {
        let ops = |st: &TargetState| target_row(st).1.into_iter().map(|(op, label)| (op, label)).collect::<Vec<_>>();
        assert_eq!(target_row(&TargetState::Unchecked).0, "まだ調べていません");
        assert_eq!(ops(&TargetState::Unchecked), [(HookOp::Check, "調べる")]);
        assert_eq!(target_row(&TargetState::Checking).0, "調べています…");
        assert!(ops(&TargetState::Checking).is_empty());
        let reg = TargetState::Known(HookStatus::Registered);
        assert_eq!(target_row(&reg).0, "登録済み");
        assert_eq!(ops(&reg), [(HookOp::Install, "登録し直す"), (HookOp::Uninstall, "外す")]);
        let not = TargetState::Known(HookStatus::NotRegistered);
        assert_eq!(target_row(&not).0, "まだ登録していません");
        assert_eq!(ops(&not), [(HookOp::Install, "登録する")]);
        let old = TargetState::Known(HookStatus::Outdated);
        assert_eq!(target_row(&old).0, "登録し直してください");
        assert_eq!(ops(&old), [(HookOp::Install, "登録し直す"), (HookOp::Uninstall, "外す")]);
        let bad = TargetState::Known(HookStatus::Unreadable("x".into()));
        assert_eq!(target_row(&bad).0, "読めません: x");
        assert_eq!(ops(&bad), [(HookOp::Check, "調べ直す")]);
        assert_eq!(target_row(&TargetState::NotInstalled).0, "まだ登録していません（登録するときに fude を WSL に置きます）");
        assert_eq!(ops(&TargetState::NotInstalled), [(HookOp::Install, "登録する")]);
    }

    #[test]
    fn reopening_rechecks_this_pc_and_checked_wsl() {
        let targets = vec![
            (HookTarget::ThisPc, TargetState::Known(HookStatus::NotRegistered)),
            (HookTarget::Wsl("A".into()), TargetState::Unchecked),
            (HookTarget::Wsl("B".into()), TargetState::Known(HookStatus::Registered)),
            (HookTarget::Wsl("C".into()), TargetState::NotInstalled),
            (HookTarget::Wsl("D".into()), TargetState::Checking),
        ];
        assert_eq!(
            rows_to_recheck(&targets),
            [HookTarget::ThisPc, HookTarget::Wsl("B".into()), HookTarget::Wsl("C".into())]
        );
    }
}
