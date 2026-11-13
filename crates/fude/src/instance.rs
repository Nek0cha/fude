//! fude を 1 つだけ動かす。起動中の fude があれば要求を渡し、無ければ自分が待ち受ける。

use std::time::Duration;

use fude_ipc::{BindError, Message, SendError, Server, send};

use crate::cli::Launch;

/// 起動中の fude の応答を待つ時間。
const SEND_TIMEOUT: Duration = Duration::from_secs(3);

pub enum HandOff {
    /// 起動中の fude に渡した。このプロセスは終わる。
    Delivered,
    /// 起動中の fude は無い。このプロセスが待ち受ける。
    Serve(Server),
}

fn message_of(launch: &Launch) -> Message {
    match launch {
        Launch::Activate => Message::Activate,
        Launch::Open { root, shell } => Message::Open { root: root.clone(), shell: shell.clone() },
        Launch::OpenFile { file, root_hint } => Message::OpenFile { file: file.clone(), root_hint: root_hint.clone() },
    }
}

/// 送る → NotRunning なら bind → AlreadyRunning（ほぼ同時に起動した相手が先に待ち受けた）ならもう一度送る。
pub fn hand_off(name: &str, launch: &Launch) -> Result<HandOff, String> {
    let msg = message_of(launch);
    match send(name, &msg, SEND_TIMEOUT) {
        Ok(()) => return Ok(HandOff::Delivered),
        Err(SendError::NotRunning) => {}
        Err(e) => return Err(e.to_string()),
    }
    match Server::bind(name) {
        Ok(server) => Ok(HandOff::Serve(server)),
        Err(BindError::AlreadyRunning) => {
            send(name, &msg, SEND_TIMEOUT).map(|()| HandOff::Delivered).map_err(|e| e.to_string())
        }
        Err(BindError::Io(e)) => Err(format!("待ち受けを始められない: {e}")),
    }
}

/// Windows のリリース版（GUI サブシステム）では標準エラーが見えないため、親のコンソールにつないでから出す。
pub fn report_error(message: &str) {
    #[cfg(windows)]
    // SAFETY: 引数は定数で、失敗しても（親にコンソールが無い、既にある）害は無い。
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    eprintln!("fude: {message}");
}
