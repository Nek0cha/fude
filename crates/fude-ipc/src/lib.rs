//! 起動中の fude に要求を渡すプロセス間通信。1 行 1 JSON でやり取りする。
//! Windows は名前付きパイプ（`\\.\pipe\<name>`）、macOS は Unix ドメインソケット（`$TMPDIR/<name>.sock`）。

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::mpsc;
use std::time::Duration;

use fude_core::workspace::ProjectRoot;
use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream, prelude::*};
use serde::{Deserialize, Serialize};

/// 1 行の上限。これを超える要求は読まずに断る。
const MAX_LINE: u64 = 64 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    /// プロジェクトを開いて（開いていれば切り替えて）前面に出す。
    Open { root: ProjectRoot },
    /// 前面に出すだけ。
    Activate,
}

#[derive(Serialize, Deserialize, Debug)]
struct Reply {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// 待ち受けの名前。"fude-<ユーザー名>"、FUDE_INSTANCE=x なら "fude-x-<ユーザー名>"。
/// ユーザー名は USERNAME、無ければ USER、無ければ "user"。英数字と - _ 以外は _ にする。
pub fn endpoint_name(env: &dyn Fn(&str) -> Option<String>) -> String {
    let user = ["USERNAME", "USER"]
        .iter()
        .find_map(|k| env(k).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "user".to_string());
    let raw = match env("FUDE_INSTANCE").filter(|v| !v.is_empty()) {
        Some(instance) => format!("fude-{instance}-{user}"),
        None => format!("fude-{user}"),
    };
    raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// Unix のソケットのファイルの場所。$TMPDIR（macOS ではユーザーごと）、無ければ /tmp。
#[cfg(unix)]
pub fn socket_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::var_os("TMPDIR").filter(|v| !v.is_empty()).unwrap_or_else(|| "/tmp".into());
    std::path::PathBuf::from(dir).join(format!("{name}.sock"))
}

/// 名前から interprocess の Name を作る。
pub fn local_name(name: &str) -> io::Result<Name<'static>> {
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        name.to_string().to_ns_name::<GenericNamespaced>()
    }
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        socket_path(name).to_fs_name::<GenericFilePath>()
    }
}

#[derive(Debug)]
pub enum SendError {
    /// 待ち受けている fude が無い。
    NotRunning,
    /// 応答が timeout までに来ない。
    Timeout,
    /// 相手が要求を断った。
    Rejected(String),
    Io(io::Error),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::NotRunning => write!(f, "起動中の fude が無い"),
            SendError::Timeout => write!(f, "起動中の fude が応答しない"),
            SendError::Rejected(e) => write!(f, "起動中の fude が要求を断った: {e}"),
            SendError::Io(e) => write!(f, "起動中の fude と通信できない: {e}"),
        }
    }
}

/// 接続の失敗が「相手がいない」ことを表すか。
fn is_absent(err: &io::Error) -> bool {
    matches!(err.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)
}

/// 1 行の JSON を送り、応答の 1 行を timeout まで待つ。
pub fn send(name: &str, msg: &Message, timeout: Duration) -> Result<(), SendError> {
    let local = local_name(name).map_err(SendError::Io)?;
    let mut line = serde_json::to_string(msg).expect("Message は必ず JSON にできる");
    line.push('\n');
    // 接続、書き込み、応答の読み取りのすべてを別のスレッドで行い、timeout で見切る。
    // interprocess の Windows の接続は、パイプが使用中だと期限なしで待ち、Stream には読み取りの
    // タイムアウトも無いため。見切ったスレッドは、プロセスの終了とともに消える。
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<String, SendError> {
            let stream = Stream::connect(local).map_err(|e| if is_absent(&e) { SendError::NotRunning } else { SendError::Io(e) })?;
            let mut reader = BufReader::new(stream);
            reader.get_mut().write_all(line.as_bytes()).map_err(SendError::Io)?;
            let mut reply = String::new();
            reader.take(MAX_LINE).read_line(&mut reply).map_err(SendError::Io)?;
            Ok(reply)
        })();
        tx.send(result).ok();
    });
    let reply = match rx.recv_timeout(timeout) {
        Ok(result) => result?,
        Err(_) => return Err(SendError::Timeout),
    };
    match serde_json::from_str::<Reply>(reply.trim()) {
        Ok(Reply { ok: true, .. }) => Ok(()),
        Ok(Reply { error, .. }) => Err(SendError::Rejected(error.unwrap_or_default())),
        Err(e) => Err(SendError::Rejected(format!("応答を読めない: {e}"))),
    }
}

#[derive(Debug)]
pub enum BindError {
    /// 他の fude が待ち受けている。
    AlreadyRunning,
    Io(io::Error),
}

pub struct Server {
    listener: Listener,
}

impl Server {
    /// 待ち受けを始める。他の fude が待ち受けていれば Err(AlreadyRunning)。
    /// Unix で、相手のいないソケットのファイルが残っていれば、消して作り直す。
    pub fn bind(name: &str) -> Result<Server, BindError> {
        let local = local_name(name).map_err(BindError::Io)?;
        match ListenerOptions::new().name(local.clone()).create_sync() {
            Ok(listener) => Ok(Server { listener }),
            Err(e) if cfg!(windows) && matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::AddrInUse) => {
                Err(BindError::AlreadyRunning)
            }
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                if Stream::connect(local.clone()).is_ok() {
                    return Err(BindError::AlreadyRunning);
                }
                #[cfg(unix)]
                std::fs::remove_file(socket_path(name)).map_err(BindError::Io)?;
                let listener = ListenerOptions::new().name(local).create_sync().map_err(BindError::Io)?;
                Ok(Server { listener })
            }
            Err(e) => Err(BindError::Io(e)),
        }
    }

    /// 受け付けのスレッドを起こし、届いた Message ごとに handler を呼ぶ（そのスレッドで呼ぶ）。
    /// 1 つの接続の処理が終わるまで、次の接続は受け付けない。
    pub fn serve(self, handler: impl Fn(Message) + Send + 'static) {
        std::thread::Builder::new()
            .name("fude-ipc".into())
            .spawn(move || {
                for conn in self.listener.incoming() {
                    match conn {
                        Ok(stream) => {
                            if let Err(e) = handle_conn(stream, &handler) {
                                eprintln!("IPC の要求を処理できない: {e}");
                            }
                        }
                        Err(e) => eprintln!("IPC の接続を受け付けられない: {e}"),
                    }
                }
            })
            .expect("IPC のスレッドを起こせない");
    }
}

/// 1 行読み、Message なら handler を呼んで ok を返す。読めなければ ok=false を返す。
fn handle_conn(stream: Stream, handler: &dyn Fn(Message)) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    (&mut reader).take(MAX_LINE).read_line(&mut line)?;
    let reply = if !line.ends_with('\n') {
        Reply { ok: false, error: Some("要求が改行で終わっていないか、長すぎる".into()) }
    } else {
        match serde_json::from_str::<Message>(line.trim()) {
            Ok(msg) => {
                handler(msg);
                Reply { ok: true, error: None }
            }
            Err(e) => Reply { ok: false, error: Some(format!("要求を読めない: {e}")) },
        }
    };
    let mut out = serde_json::to_string(&reply).expect("Reply は必ず JSON にできる");
    out.push('\n');
    reader.get_mut().write_all(out.as_bytes())?;
    reader.get_mut().flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn name_with(vars: &[(&str, &str)]) -> String {
        let map: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        endpoint_name(&|k| map.get(k).cloned())
    }

    #[test]
    fn endpoint_name_rules() {
        assert_eq!(name_with(&[("USERNAME", "neko")]), "fude-neko");
        assert_eq!(name_with(&[("USER", "neko")]), "fude-neko");
        assert_eq!(name_with(&[("USERNAME", "neko"), ("FUDE_INSTANCE", "dev")]), "fude-dev-neko");
        assert_eq!(name_with(&[("USERNAME", "a b/c")]), "fude-a_b_c");
        assert_eq!(name_with(&[]), "fude-user");
    }
}
