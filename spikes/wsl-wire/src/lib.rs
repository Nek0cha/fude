//! 検証4：WSL 側のサーバーと、標準入出力で 1 行 1 JSON のやり取りをする。

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

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
        output.write_all(b"
")?;
        output.flush()?;
    }
    Ok(())
}

/// 起動したサーバーとの接続。
pub struct ServerConn {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// `wsl.exe -d <distro> -- <bin> serve` を起動する。
pub fn spawn_server(distro: &str, bin: &str) -> io::Result<ServerConn> {
    let mut child = Command::new("wsl.exe")
        .args(["-d", distro, "--", bin, "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdin = child.stdin.take().expect("stdin はパイプにしてある");
    let stdout = BufReader::new(child.stdout.take().expect("stdout はパイプにしてある"));
    Ok(ServerConn { child, stdin, stdout })
}

impl ServerConn {
    /// 要求を送り、応答を 1 つ受け取る。サーバーが終了していたらエラーを返す。
    pub fn call(&mut self, req: &Request) -> io::Result<Response> {
        serde_json::to_writer(&mut self.stdin, req)?;
        self.stdin.write_all(b"
")?;
        self.stdin.flush()?;
        let mut line = String::new();
        if self.stdout.read_line(&mut line)? == 0 {
            let status = self.child.wait()?;
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, format!("サーバーが終了した: {status}")));
        }
        serde_json::from_str(&line).map_err(io::Error::other)
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
    fn missing_distro_reports_error() {
        let res = spawn_server("NoSuchDistro", "/bin/true").and_then(|mut c| c.call(&Request::Ping { id: 1 }));
        assert!(res.is_err(), "{res:?}");
    }
}
