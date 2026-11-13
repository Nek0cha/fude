//! 本物のシェルを PTY で動かす結合テスト。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use fude_term::{GridSize, Session, SessionEvent, SpawnSpec, Snapshot, TermState};

const SIZE: GridSize = GridSize { cols: 80, rows: 24 };
const TIMEOUT: Duration = Duration::from_secs(20);

fn palette() -> fude_term::Palette {
    fude_term::Palette { ansi: [0; 16], foreground: 0xffffff, background: 0, cursor: 0xffffff }
}

/// シェルに 1 つのコマンド文字列を実行させる。
fn shell(command: &str) -> SpawnSpec {
    let (program, flag) = if cfg!(windows) { ("cmd.exe", "/c") } else { ("sh", "-c") };
    SpawnSpec {
        program: program.into(),
        args: vec![flag.into(), command.into()],
        cwd: std::env::current_dir().unwrap(),
        env: Vec::new(),
    }
}

fn spawn(spec: &SpawnSpec) -> (Session, Receiver<SessionEvent>) {
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let session = Session::spawn(spec, SIZE, Arc::new(move |ev| {
        let _ = tx.lock().unwrap().send(ev);
    }))
    .expect("起動できる");
    (session, rx)
}

/// Exited を待ち、終了コードを返す。
fn wait_exit(rx: &Receiver<SessionEvent>) -> Option<u32> {
    let deadline = Instant::now() + TIMEOUT;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if let Ok(SessionEvent::Exited { code }) = rx.recv_timeout(left) {
            return code;
        }
    }
    panic!("終了しない");
}

fn screen(s: &Session) -> String {
    let snap: Snapshot = s.with_state(|st: &mut TermState| st.snapshot(&palette(), true));
    (0..snap.rows).map(|r| snap.row_text(r)).collect::<Vec<_>>().join("\n")
}

/// 画面に needle が出るまで待つ。
fn wait_screen(s: &Session, needle: &str) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if screen(s).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn echo_appears_in_snapshot() {
    let (s, rx) = spawn(&shell("echo fude-ok"));
    wait_exit(&rx);
    assert!(wait_screen(&s, "fude-ok"), "{}", screen(&s));
}

#[test]
fn exit_code_is_reported() {
    let (_s, rx) = spawn(&shell("exit 3"));
    assert_eq!(wait_exit(&rx), Some(3));
}

#[test]
fn spawn_missing_program_errors() {
    let spec = SpawnSpec { program: "fude-no-such-program".into(), ..shell("") };
    let err = Session::spawn(&spec, SIZE, Arc::new(|_| {})).err().expect("失敗する");
    assert!(err.to_string().contains("fude-no-such-program"), "{err}");
}

#[test]
fn write_reaches_child() {
    let spec = if cfg!(windows) {
        SpawnSpec {
            program: "powershell.exe".into(),
            args: vec!["-NoProfile".into(), "-Command".into(), "$x = Read-Host 'ready'; Write-Output \"got-$x\"".into()],
            ..shell("")
        }
    } else {
        shell("printf ready; read x; printf got-$x")
    };
    let (s, _rx) = spawn(&spec);
    assert!(wait_screen(&s, "ready"), "{}", screen(&s));
    s.write(b"abc\r".to_vec());
    assert!(wait_screen(&s, "got-abc"), "{}", screen(&s));
}

#[test]
fn resize_changes_state_size() {
    let (s, rx) = spawn(&shell("echo x"));
    s.resize(GridSize { cols: 50, rows: 10 });
    assert_eq!(s.with_state(|st| st.size()), GridSize { cols: 50, rows: 10 });
    wait_exit(&rx);
}

#[test]
fn large_output_keeps_state_responsive() {
    let (command, last) = if cfg!(windows) {
        ("for /L %i in (1,1,20000) do @echo line %i", "line 20000")
    } else {
        ("seq 1 200000", "200000")
    };
    let (s, rx) = spawn(&shell(command));
    let mut worst = Duration::ZERO;
    loop {
        let start = Instant::now();
        s.with_state(|_| ());
        worst = worst.max(start.elapsed());
        if let Ok(SessionEvent::Exited { .. }) = rx.recv_timeout(Duration::from_millis(10)) {
            break;
        }
    }
    assert!(worst < Duration::from_millis(100), "ロック待ちの最大: {worst:?}");
    assert!(wait_screen(&s, last), "{}", screen(&s));
}
