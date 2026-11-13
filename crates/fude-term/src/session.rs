//! PTY のセッション。シェルを起動し、出力を TermState に流し込む。
//!
//! スレッドは 3 つ使う。
//! - 読み取り: PTY の出力を解析し、UI に Wakeup を送る。DSR の応答などは書き込みスレッドへ回す。
//! - 書き込み: PTY への書き込みだけを行う。読み取りスレッドが書き込みで詰まらないようにするため。
//! - 終了待ち: 子プロセスの終了を待ち、Exited を送る。

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use alacritty_terminal::sync::FairMutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::shell::SpawnSpec;
use crate::state::{GridSize, TermEvent, TermState};

/// 遡れる行数。
const SCROLLBACK: usize = 10_000;
/// 読み取りの 1 回の上限。ロックを持つ時間を短く保つ。
const READ_CHUNK: usize = 64 * 1024;
/// 終了を知らせる前に、残りの出力を読み終えるのを待つ時間。
const DRAIN_WAIT: Duration = Duration::from_millis(500);

/// セッションから UI へ知らせること。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// 画面が変わった。描き直す。
    Wakeup,
    /// OSC の題名。None は既定に戻す。
    Title(Option<String>),
    /// 子プロセスが終わった。終了コードが取れなければ None。
    Exited { code: Option<u32> },
}

type OnEvent = Arc<dyn Fn(SessionEvent) + Send + Sync>;

pub struct Session {
    /// 読み取りスレッドが取り直し続けても UI の描画が待たされないよう、順番待ちが公平なロックを使う。
    state: Arc<FairMutex<TermState>>,
    writer: Sender<Vec<u8>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

fn pty_size(size: GridSize) -> PtySize {
    PtySize { rows: size.rows, cols: size.cols, pixel_width: 0, pixel_height: 0 }
}

impl Session {
    pub fn spawn(spec: &SpawnSpec, size: GridSize, on_event: OnEvent) -> io::Result<Session> {
        let fail = |what: &str, err: &dyn std::fmt::Display| io::Error::other(format!("{} を{what}できない: {err}", spec.program));
        let pair = native_pty_system().openpty(pty_size(size)).map_err(|e| fail("起動する PTY を用意", &e))?;
        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| fail("起動", &e))?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().map_err(|e| fail("読み取り", &e))?;
        let mut pty_writer = pair.master.take_writer().map_err(|e| fail("書き込み", &e))?;

        let state = Arc::new(FairMutex::new(TermState::new(size, SCROLLBACK)));
        let (writer, writes) = mpsc::channel::<Vec<u8>>();

        thread::Builder::new().name("fude-pty-write".into()).spawn(move || {
            for bytes in writes {
                if pty_writer.write_all(&bytes).and_then(|()| pty_writer.flush()).is_err() {
                    break;
                }
            }
        })?;

        let (read_done, read_done_rx) = mpsc::channel::<()>();
        {
            let state = state.clone();
            let writer = writer.clone();
            let on_event = on_event.clone();
            thread::Builder::new().name("fude-pty-read".into()).spawn(move || {
                let mut buf = vec![0u8; READ_CHUNK];
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let events = {
                        let mut st = state.lock();
                        st.advance(&buf[..n]);
                        st.take_events()
                    };
                    for ev in events {
                        match ev {
                            TermEvent::Write(bytes) => {
                                let _ = writer.send(bytes);
                            }
                            TermEvent::Title(title) => on_event(SessionEvent::Title(title)),
                        }
                    }
                    on_event(SessionEvent::Wakeup);
                }
                let _ = read_done.send(());
            })?;
        }

        thread::Builder::new().name("fude-pty-wait".into()).spawn(move || {
            let code = child.wait().ok().map(|status| status.exit_code());
            // Windows の ConPTY は子が終わっても読み取りが終わらないことがあるため、待つのは一定時間だけ。
            let _ = read_done_rx.recv_timeout(DRAIN_WAIT);
            on_event(SessionEvent::Exited { code });
        })?;

        Ok(Session { state, writer, master: Mutex::new(pair.master), killer: Mutex::new(killer) })
    }

    /// 入力を送る。表示を一番下へ戻し、選択を消す。
    pub fn write(&self, bytes: Vec<u8>) {
        self.with_state(|st| {
            st.scroll_to_bottom();
            st.clear_selection();
        });
        let _ = self.writer.send(bytes);
    }

    /// 大きさが変わったときだけ、TermState と PTY の両方を変える。
    pub fn resize(&self, size: GridSize) {
        let changed = self.with_state(|st| {
            let changed = st.size() != size;
            st.resize(size);
            changed
        });
        if changed {
            let _ = self.master.lock().expect("PTY のロック").resize(pty_size(size));
        }
    }

    pub fn with_state<R>(&self, f: impl FnOnce(&mut TermState) -> R) -> R {
        f(&mut self.state.lock())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.killer.lock().expect("子プロセスのロック").kill();
    }
}
