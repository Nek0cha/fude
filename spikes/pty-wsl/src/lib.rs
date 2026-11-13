//! 検証1：PTY を通したキー入力のバイト列の往復を確かめる。

use std::io::{self, Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// 受け手が入力を受け付ける準備ができたことを示す文字列。受け手のコマンドが出力する。
pub const READY_MARKER: &[u8] = b"FUDE-READY";
/// プロセスの終了後、残りの出力を受け取るために待つ時間。
const DRAIN_WAIT: Duration = Duration::from_millis(300);

/// PTY でコマンドを起動し、出力に [`READY_MARKER`] が現れてから `input` を書き込む。
/// プロセスが終わるかタイムアウトするまでの出力を返す。
pub fn roundtrip(cmd: CommandBuilder, input: &[u8], timeout: Duration) -> io::Result<String> {
    let pair = native_pty_system()
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(io::Error::other)?;
    let mut child = pair.slave.spawn_command(cmd).map_err(io::Error::other)?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut writer = pair.master.take_writer().map_err(io::Error::other)?;
    let mut output = Vec::new();
    let mut answered = 0;
    // ConPTY は起動時にカーソル位置の問い合わせ（DSR, `ESC [ 6 n`）を送り、応答があるまで止まる。
    // 本物のターミナルと同じく、問い合わせのたびに位置（1 行 1 列）を返す。
    let mut pump = |output: &mut Vec<u8>, writer: &mut Box<dyn Write + Send>| -> io::Result<()> {
        while let Ok(chunk) = rx.try_recv() {
            output.extend(chunk);
        }
        let asked = output.windows(4).filter(|w| *w == b"\x1b[6n").count();
        for _ in answered..asked {
            writer.write_all(b"\x1b[1;1R")?;
            writer.flush()?;
        }
        answered = asked;
        Ok(())
    };

    let deadline = Instant::now() + timeout;
    while !output.windows(READY_MARKER.len()).any(|w| w == READY_MARKER) {
        if Instant::now() >= deadline || child.try_wait()?.is_some() {
            let _ = child.kill();
            return Ok(String::from_utf8_lossy(&output).into_owned());
        }
        pump(&mut output, &mut writer)?;
        thread::sleep(Duration::from_millis(20));
    }
    writer.write_all(input)?;
    writer.flush()?;

    loop {
        pump(&mut output, &mut writer)?;
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let drain_until = Instant::now() + DRAIN_WAIT;
    while let Some(left) = drain_until.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(chunk) => output.extend(chunk),
            Err(_) => break,
        }
    }
    drop(writer);
    drop(pair.master);
    Ok(String::from_utf8_lossy(&output).into_owned())
}
