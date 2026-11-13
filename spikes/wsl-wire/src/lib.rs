//! 検証4：WSL 側のサーバーと、標準入出力で 1 行 1 JSON のやり取りをする。

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::io::Read;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// クライアントからサーバーへの要求。
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Ping { id: u64 },
    /// `root` 以下のファイルを、.gitignore を尊重して数える。
    Walk { id: u64, root: String },
}

/// サーバーからクライアントへの応答。
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Pong { id: u64 },
    Walked { id: u64, count: u64, ms: u64 },
    Error { id: u64, message: String },
}

/// サーバー側で要求を 1 つ処理する。
pub fn handle(req: Request) -> Response {
    match req {
        Request::Ping { id } => Response::Pong { id },
        Request::Walk { id, root } => match walk_count(Path::new(&root)) {
            Ok((count, ms)) => Response::Walked { id, count, ms },
            Err(message) => Response::Error { id, message },
        },
    }
}

/// `root` 以下のファイル数と、数えるのにかかった時間（ミリ秒）を返す。`.git` ディレクトリは数えない。
pub fn walk_count(root: &Path) -> Result<(u64, u64), String> {
    if !root.is_dir() {
        return Err(format!("ディレクトリがない: {}", root.display()));
    }
    let start = Instant::now();
    let count = ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .count() as u64;
    Ok((count, start.elapsed().as_millis() as u64))
}

/// 標準入力から 1 行ずつ要求を読み、応答を標準出力に書く。入力が終わったら戻る。
pub fn serve(input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        let res = match serde_json::from_str::<Request>(&line) {
            Ok(req) => handle(req),
            Err(e) => Response::Error { id: 0, message: format!("要求を解釈できない: {e}") },
        };
        serde_json::to_writer(&mut output, &res)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

/// 起動したサーバーとの接続。
pub struct ServerConn {
    child: Child,
    stdin: ChildStdin,
    /// stdout を別スレッドで 1 行ずつ読んだ結果。サーバーが終了すると切断される。
    lines: mpsc::Receiver<String>,
    /// サーバー（と wsl.exe）が stderr に書いた内容。エラーの説明に使う。
    stderr: Arc<Mutex<String>>,
    /// 応答を待つ時間の上限。
    timeout: Duration,
}

/// 応答を待つ時間の既定値。WSL の起動が遅いときでも足りる長さにする。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// stdout を 1 行ずつ読み、チャネルに送るスレッドを起動する。
fn spawn_line_reader(stdout: ChildStdout) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// wsl.exe の出力を文字列にする。wsl.exe は既定で UTF-16LE を出すため、NUL を含むときは UTF-16LE として読む。
fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.contains(&0) {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// インストール済みの WSL ディストリビューションの名前を返す。
pub fn list_distros() -> io::Result<Vec<String>> {
    let out = Command::new("wsl.exe").args(["-l", "-q"]).env("WSL_UTF8", "1").output()?;
    Ok(decode_wsl_output(&out.stdout)
        .lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\0').to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// `wsl.exe -d <distro> -- <bin> serve` を起動する。
/// ディストリビューションが無いときは、インストール済みの名前を添えて `NotFound` を返す。
pub fn spawn_server(distro: &str, bin: &str) -> io::Result<ServerConn> {
    let installed = list_distros()?;
    if !installed.iter().any(|d| d == distro) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("WSL のディストリビューション '{distro}' が見つからない（インストール済み: {}）", installed.join(", ")),
        ));
    }
    let mut child = Command::new("wsl.exe")
        .args(["-d", distro, "--", bin, "serve"])
        .env("WSL_UTF8", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take().expect("stdin はパイプにしてある");
    let lines = spawn_line_reader(child.stdout.take().expect("stdout はパイプにしてある"));
    let mut err_pipe = child.stderr.take().expect("stderr はパイプにしてある");
    let stderr = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&stderr);
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        *sink.lock().unwrap() = decode_wsl_output(&buf);
    });
    Ok(ServerConn { child, stdin, lines, stderr, timeout: DEFAULT_TIMEOUT })
}

impl ServerConn {
    /// 応答を待つ時間の上限を変える。
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// 要求を送り、応答を 1 つ受け取る。
    /// 時間内に応答が無ければ `TimedOut` を返す。
    /// サーバーが終了していたら、終了コードと stderr の内容を添えたエラーを返す。
    pub fn call(&mut self, req: &Request) -> io::Result<Response> {
        let sent = serde_json::to_writer(&mut self.stdin, req)
            .map_err(io::Error::other)
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush());
        if sent.is_err() {
            return Err(self.exited_error());
        }
        match self.lines.recv_timeout(self.timeout) {
            Ok(line) => serde_json::from_str(&line).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("サーバーの応答を解釈できない（{e}）: {}", line.trim()))
            }),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("サーバーが {} 秒以内に応答しない", self.timeout.as_secs_f32()),
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(self.exited_error()),
        }
    }

    fn exited_error(&mut self) -> io::Error {
        let status = self.child.wait().map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
        // stderr を読むスレッドが書き終えるのを少し待つ。
        std::thread::sleep(std::time::Duration::from_millis(100));
        let stderr = self.stderr.lock().unwrap().trim().to_string();
        io::Error::new(io::ErrorKind::UnexpectedEof, format!("サーバーが終了した（{status}）。stderr: {stderr}"))
    }
}

impl Drop for ServerConn {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::fs;

    #[test]
    fn ping_returns_pong() {
        assert_eq!(handle(Request::Ping { id: 7 }), Response::Pong { id: 7 });
    }

    #[test]
    fn walk_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        fs::write(root.join("a.rs"), "").unwrap();
        fs::create_dir(root.join("target")).unwrap();
        fs::write(root.join("target/b.rs"), "").unwrap();
        let Response::Walked { count, .. } = handle(Request::Walk { id: 1, root: root.to_string_lossy().into_owned() }) else {
            panic!("Walked が返らない");
        };
        assert_eq!(count, 2);
    }

    #[test]
    fn walk_missing_root_is_error() {
        let res = handle(Request::Walk { id: 2, root: "/no/such/dir".into() });
        assert!(matches!(res, Response::Error { id: 2, .. }), "{res:?}");
    }

    #[test]
    fn request_json_shape() {
        assert_eq!(serde_json::to_string(&Request::Ping { id: 1 }).unwrap(), r#"{"op":"ping","id":1}"#);
    }

    #[cfg(windows)]
    #[test]
    fn unresponsive_server_times_out() {
        // 応答を返さずに眠り続けるサーバーの代わり。
        let ok = std::process::Command::new("wsl.exe")
            .args(["-d", "Ubuntu", "-e", "sh", "-c", "printf '#!/bin/sh\\nsleep 30\\n' > /tmp/fude-hang && chmod +x /tmp/fude-hang"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let mut conn = spawn_server("Ubuntu", "/tmp/fude-hang").unwrap();
        conn.set_timeout(std::time::Duration::from_secs(1));
        let start = Instant::now();
        let err = conn.call(&Request::Ping { id: 1 }).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }

    #[cfg(windows)]
    #[test]
    fn missing_distro_reports_error() {
        let err = spawn_server("NoSuchDistro", "/bin/true")
            .and_then(|mut c| c.call(&Request::Ping { id: 1 }))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound, "{err}");
        assert!(err.to_string().contains("NoSuchDistro"), "{err}");
        assert!(err.to_string().contains("Ubuntu"), "インストール済みの名前を示す: {err}");
    }

    #[cfg(windows)]
    #[test]
    fn missing_bin_reports_exit_code_and_stderr() {
        let err = spawn_server("Ubuntu", "/no/such/bin")
            .and_then(|mut c| c.call(&Request::Ping { id: 1 }))
            .unwrap_err();
        assert!(err.to_string().contains("127"), "{err}");
        assert!(err.to_string().contains("/no/such/bin"), "stderr の内容を含める: {err}");
    }
}
