//! 本物のパイプ／ソケットを使う結合テスト。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fude_core::workspace::ProjectRoot;
use fude_ipc::{BindError, Message, SendError, Server, local_name, send};
use interprocess::local_socket::{ListenerOptions, Stream, prelude::*};

const TIMEOUT: Duration = Duration::from_secs(3);

fn name(test: &str) -> String {
    format!("fude-test-{}-{test}", std::process::id())
}

/// 待ち受けを始め、届いた Message をチャネルに流す。
fn serve(name: &str) -> mpsc::Receiver<Message> {
    let (tx, rx) = mpsc::channel();
    let server = Server::bind(name).unwrap_or_else(|_| panic!("待ち受けられる"));
    server.serve(move |m| {
        tx.send(m).ok();
    });
    rx
}

#[test]
fn send_without_server_is_not_running() {
    let result = send(&name("none"), &Message::Activate, TIMEOUT);
    assert!(matches!(result, Err(SendError::NotRunning)), "{result:?}");
}

#[test]
fn message_roundtrip() {
    let n = name("roundtrip");
    let rx = serve(&n);
    let open = Message::Open { root: ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x".into() } };
    send(&n, &open, TIMEOUT).unwrap();
    send(&n, &Message::Activate, TIMEOUT).unwrap();
    let local = Message::Open { root: ProjectRoot::Local(PathBuf::from(r"C:\w\a")) };
    send(&n, &local, TIMEOUT).unwrap();
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), open);
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), Message::Activate);
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), local);
}

#[test]
fn second_bind_is_already_running() {
    let n = name("second");
    let _rx = serve(&n);
    assert!(matches!(Server::bind(&n), Err(BindError::AlreadyRunning)));
}

#[test]
fn garbage_line_is_rejected_and_server_keeps_serving() {
    let n = name("garbage");
    let rx = serve(&n);
    let stream = Stream::connect(local_name(&n).unwrap()).unwrap();
    let mut reader = BufReader::new(stream);
    reader.get_mut().write_all(b"not json\n").unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let reply: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["ok"], false);
    drop(reader);
    send(&n, &Message::Activate, TIMEOUT).unwrap();
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), Message::Activate);
}

#[test]
fn unresponsive_server_times_out() {
    let n = name("silent");
    let listener = ListenerOptions::new().name(local_name(&n).unwrap()).create_sync().unwrap();
    std::thread::spawn(move || {
        // 受け付けるが、応答しない。
        let mut held = Vec::new();
        for conn in listener.incoming() {
            held.push(conn);
        }
    });
    let start = Instant::now();
    let result = send(&n, &Message::Activate, Duration::from_millis(300));
    assert!(matches!(result, Err(SendError::Timeout)), "{result:?}");
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn many_clients_all_delivered() {
    let n = name("many");
    let rx = serve(&n);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let n = n.clone();
            std::thread::spawn(move || send(&n, &Message::Activate, TIMEOUT))
        })
        .collect();
    for t in threads {
        t.join().unwrap().unwrap();
    }
    for _ in 0..8 {
        rx.recv_timeout(TIMEOUT).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn stale_socket_file_is_reclaimed() {
    let n = name("stale");
    let path = fude_ipc::socket_path(&n);
    let _ = std::fs::remove_file(&path);
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists(), "ソケットのファイルが残っている");
    let rx = serve(&n);
    send(&n, &Message::Activate, TIMEOUT).unwrap();
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), Message::Activate);
}
