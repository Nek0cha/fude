//! PTY を通して送ったバイト列が、受け手のプロセスに変わらずに届くかを確かめる。
//! 受け手は端末を raw モードにして N バイトだけ読み、16 進数で表示する。

use std::time::Duration;

use fude_core::keys::{KeyCode, KeyInput, KeyOverrides, Modifiers, TermModes, encode_key};
use portable_pty::CommandBuilder;
use pty_wsl::roundtrip;

fn probe(n: usize) -> String {
    format!("stty raw -echo; printf FUDE-READY; head -c {n} | od -An -tx1")
}

#[cfg(windows)]
fn wsl(n: usize) -> CommandBuilder {
    let mut cmd = CommandBuilder::new("wsl.exe");
    cmd.args(["-d", "Ubuntu", "--", "bash", "-c", &probe(n)]);
    cmd
}

#[cfg(unix)]
fn sh(n: usize) -> CommandBuilder {
    let mut cmd = CommandBuilder::new("sh");
    cmd.args(["-c", &probe(n)]);
    cmd
}

fn shift_enter_bytes() -> Vec<u8> {
    let key = KeyInput { code: KeyCode::Enter, mods: Modifiers { shift: true, ..Modifiers::default() } };
    encode_key(&key, TermModes::default(), &KeyOverrides::defaults()).unwrap()
}

#[cfg(windows)]
#[test]
#[ignore = "WSL の Ubuntu が必要"]
fn shift_enter_reaches_wsl_unchanged() {
    let out = roundtrip(wsl(2), &shift_enter_bytes(), Duration::from_secs(10)).unwrap();
    assert!(out.contains("1b 0d"), "{out:?}");
}

#[cfg(windows)]
#[test]
#[ignore = "WSL の Ubuntu が必要"]
fn cjk_reaches_wsl_unchanged() {
    let out = roundtrip(wsl(3), "あ".as_bytes(), Duration::from_secs(10)).unwrap();
    assert!(out.contains("e3 81 82"), "{out:?}");
}

#[cfg(unix)]
#[test]
fn shift_enter_reaches_shell_unchanged() {
    let out = roundtrip(sh(2), &shift_enter_bytes(), Duration::from_secs(10)).unwrap();
    assert!(out.contains("1b 0d"), "{out:?}");
}
