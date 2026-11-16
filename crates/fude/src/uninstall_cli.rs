//! `fude uninstall-cleanup`（10b の設計書 §5.4）。インストーラーのアンインストールが、ファイルを消す前に呼ぶ。
//! この PC の Claude Code の hook と、記録にある WSL の hook・fude を片付ける。設定やセッションは消さない。

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fude_core::wsl_record::Deployed;

/// 1 つの組にかける時間の上限。止まっているディストロの最初の起動（wsl_host の FIRST_TIMEOUT は 30 秒）より長くする。
pub(crate) const PAIR_TIMEOUT: Duration = Duration::from_secs(45);

/// この PC の分を行い、記録の組を同時に片付ける。timeout を過ぎた組は待たずに打ち切る。
/// 戻り値はログの行（この PC の 1 行と、組ごとに 1 行）。
pub(crate) fn clean_all(
    records: &[Deployed],
    this_pc: impl FnOnce() -> String,
    one: impl Fn(&Deployed) -> Result<(), String> + Send + Sync + 'static,
    timeout: Duration,
) -> Vec<String> {
    let one = Arc::new(one);
    let (tx, rx) = mpsc::channel();
    for (i, record) in records.iter().enumerate() {
        let (one, tx, record) = (one.clone(), tx.clone(), record.clone());
        std::thread::spawn(move || {
            let _ = tx.send((i, one(&record)));
        });
    }
    drop(tx);
    let mut log = vec![this_pc()];
    let mut results: Vec<Option<Result<(), String>>> = vec![None; records.len()];
    let deadline = Instant::now() + timeout;
    while results.iter().any(Option::is_none) {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((i, r)) => results[i] = Some(r),
            Err(_) => break,
        }
    }
    for (record, result) in records.iter().zip(results) {
        let what = match result {
            Some(Ok(())) => "片付けた".to_string(),
            Some(Err(e)) => format!("失敗: {e}"),
            None => format!("打ち切り（{} 秒）", timeout.as_secs_f32()),
        };
        log.push(format!("WSL {} ({}): {what}", record.distro, record.bin));
    }
    log
}

/// WSL の `fude hook --uninstall` の結果から、hook が外れたか（または fude が無いか）を見る。
/// 外れていないのに fude を消すと、存在しないコマンドを呼ぶ hook が残るので、そのときは Err にして fude を残す。
#[cfg(windows)]
fn wsl_hook_removed(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Result<(), String> {
    use fude_core::claude_settings::HookStatus;
    if code == Some(fude_fs::wsl::RUN_NOT_INSTALLED) {
        return Ok(());
    }
    match crate::hook_targets::parse_status_output(stdout, stderr) {
        HookStatus::NotRegistered => Ok(()),
        HookStatus::Unreadable(reason) => Err(format!("hook を外せない（{reason}）。fude は残した")),
        other => Err(format!("hook を外せない（{other:?}）。fude は残した")),
    }
}

/// 1 つの組を片付ける：そのディストロで hook を外し、外れたら fude と host-exe を消す。
#[cfg(windows)]
fn clean_one(record: &Deployed) -> Result<(), String> {
    use fude_fs::wsl::{remove_deployed, run_fude};
    let out = run_fude(&record.distro, &record.bin, &["hook", "--uninstall"], PAIR_TIMEOUT).map_err(|e| format!("hook: {e}"))?;
    wsl_hook_removed(out.status.code(), &out.stdout, &out.stderr)?;
    remove_deployed(&record.distro, &record.bin, PAIR_TIMEOUT).map_err(|e| format!("fude: {e}"))
}

#[cfg(not(windows))]
fn clean_one(_record: &Deployed) -> Result<(), String> {
    Ok(())
}

/// 終了コードは常に 0（アンインストールを止めない）。何をしたかを %TEMP%\fude-uninstall.log に書く。
pub(crate) fn uninstall_main() -> i32 {
    use fude_core::wsl_record::{load, record_path};
    // 既定のインスタンスの記録だけを見る（FUDE_INSTANCE は読まない）。
    let env = |k: &str| if k == "FUDE_INSTANCE" { None } else { std::env::var(k).ok() };
    let path = record_path(fude_core::keymap::Platform::current(), &env);
    let records = path.as_deref().map(load).unwrap_or_default();
    let mut log = clean_all(&records, crate::hook_cli::uninstall_this_pc, clean_one, PAIR_TIMEOUT);
    if let Some(path) = &path {
        match std::fs::remove_file(path) {
            Ok(()) => log.push(format!("記録を消した: {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log.push(format!("記録を消せない: {}: {e}", path.display())),
        }
    }
    let _ = std::fs::write(std::env::temp_dir().join("fude-uninstall.log"), log.join("\n") + "\n");
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use fude_core::wsl_record::Deployed;
    use std::time::{Duration, Instant};

    fn d(name: &str) -> Deployed {
        Deployed { distro: name.into(), bin: "~/.local/bin/fude".into() }
    }

    #[test]
    fn clean_all_continues_after_failure_and_timeout() {
        let recs = vec![d("A"), d("B"), d("C")];
        let start = Instant::now();
        let log = clean_all(
            &recs,
            || "この PC: 外した".into(),
            |r| match r.distro.as_str() {
                "A" => Ok(()),
                "B" => Err("ディストロが無い".into()),
                _ => {
                    std::thread::sleep(Duration::from_secs(5));
                    Ok(())
                }
            },
            Duration::from_millis(300),
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(log.len(), 4);
        assert!(log.iter().any(|l| l.contains("A") && l.contains("片付けた")), "{log:?}");
        assert!(log.iter().any(|l| l.contains("B") && l.contains("ディストロが無い")), "{log:?}");
        assert!(log.iter().any(|l| l.contains("C") && l.contains("打ち切り")), "{log:?}");
    }

    #[test]
    fn clean_all_runs_pairs_in_parallel() {
        let recs = vec![d("A"), d("B"), d("C")];
        let start = Instant::now();
        clean_all(
            &recs,
            String::new,
            |_| {
                std::thread::sleep(Duration::from_millis(200));
                Ok(())
            },
            Duration::from_secs(5),
        );
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[test]
    #[cfg(windows)]
    fn wsl_hook_must_be_removed_before_deleting_fude() {
        // 外せた（登録なし）、fude が無い（終了コード 3）なら消してよい。それ以外は fude を残す。
        assert!(wsl_hook_removed(Some(0), br#"{"status":"not_registered"}"#, b"").is_ok());
        assert!(wsl_hook_removed(Some(3), b"", b"").is_ok());
        assert!(wsl_hook_removed(Some(0), br#"{"status":"unreadable","reason":"x"}"#, b"").is_err());
        assert!(wsl_hook_removed(Some(1), b"", b"wsl: error").is_err());
        assert!(wsl_hook_removed(Some(0), br#"{"status":"registered"}"#, b"").is_err());
    }

    #[test]
    fn clean_all_with_no_records_only_cleans_this_pc() {
        assert_eq!(clean_all(&[], || "この PC".into(), |_| Ok(()), Duration::from_secs(1)).len(), 1);
    }
}
