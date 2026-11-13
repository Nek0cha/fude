//! fude serve と話すクライアント。OS に依らず、任意の読み書きの組（子プロセスの標準入出力など）で動く。

use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::wire::{Request, Response, read_frame, write_frame};
use crate::{DirEntry, ProjectFs, RelPath};

pub trait Connector: Send + Sync {
    /// サーバーを起こし、(書き込み口, 読み取り口) を返す。書き込み口を捨てたらサーバーも終わるようにする。
    fn connect(&self) -> io::Result<(Box<dyn Write + Send>, Box<dyn BufRead + Send>)>;
}

type Frame = io::Result<(Response, Vec<u8>)>;

struct Conn {
    writer: Box<dyn Write + Send>,
    /// 読み取り口から読んだ応答。読み取りは別のスレッドで行い、ここで待つ時間を区切る。
    responses: mpsc::Receiver<Frame>,
    /// まだ 1 つも要求を送っていない。
    fresh: bool,
}

/// 1 つのサーバーに要求を順に送るクライアント。応答が時間内に来ない、またはサーバーが終わっていたら、
/// その接続を捨てて Err を返し、次の要求で connect し直す。
pub struct RemoteClient {
    connector: Box<dyn Connector>,
    conn: Mutex<Option<Conn>>,
    timeout: Duration,
    first_timeout: Duration,
}

impl RemoteClient {
    pub fn new(connector: Box<dyn Connector>, timeout: Duration) -> RemoteClient {
        RemoteClient { connector, conn: Mutex::new(None), timeout, first_timeout: timeout }
    }

    /// 接続して最初の要求だけ、待つ時間を変える（サーバーの起動を待つため）。
    pub fn with_first_timeout(mut self, first_timeout: Duration) -> RemoteClient {
        self.first_timeout = first_timeout;
        self
    }

    /// 使っていた接続が切れていた（サーバーが終わっていた）ときだけ、1 回つなぎ直して送り直す。
    /// 時間切れと、つないだばかりの接続での失敗は送り直さない。
    pub fn call(&self, req: &Request, payload: &[u8]) -> io::Result<(Response, Vec<u8>)> {
        let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut retried = false;
        loop {
            let conn = match guard.as_mut() {
                Some(conn) => conn,
                None => guard.insert(self.open()?),
            };
            let was_fresh = conn.fresh;
            let timeout = if conn.fresh { self.first_timeout } else { self.timeout };
            conn.fresh = false;
            let result = exchange(conn, req, payload, timeout);
            let Err(e) = &result else { return result };
            *guard = None;
            if retried || was_fresh || e.kind() != io::ErrorKind::BrokenPipe {
                return result;
            }
            retried = true;
        }
    }

    fn open(&self) -> io::Result<Conn> {
        let (writer, mut reader) = self.connector.connect()?;
        let (tx, responses) = mpsc::channel();
        std::thread::spawn(move || {
            loop {
                match read_frame::<Response>(&mut reader) {
                    Ok(Some(frame)) => {
                        if tx.send(Ok(frame)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });
        Ok(Conn { writer, responses, fresh: true })
    }
}

fn exchange(conn: &mut Conn, req: &Request, payload: &[u8], timeout: Duration) -> Frame {
    write_frame(&mut conn.writer, req, payload)?;
    match conn.responses.recv_timeout(timeout) {
        Ok(frame) => frame,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("fude serve が {} 秒以内に応答しない", timeout.as_secs_f32()),
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "fude serve が終わった"))
        }
    }
}

/// root（サーバー側の絶対パス）に対する ProjectFs。Response::Error は io::Error（not_found なら NotFound）にする。
pub struct RemoteFs {
    client: Arc<RemoteClient>,
    root: String,
}

impl RemoteFs {
    pub fn new(client: Arc<RemoteClient>, root: String) -> RemoteFs {
        RemoteFs { client, root }
    }

    fn call(&self, req: Request, payload: &[u8]) -> io::Result<(Response, Vec<u8>)> {
        match self.client.call(&req, payload)? {
            (Response::Error { message, not_found }, _) => {
                let kind = if not_found { io::ErrorKind::NotFound } else { io::ErrorKind::Other };
                Err(io::Error::new(kind, message))
            }
            frame => Ok(frame),
        }
    }
}

fn unexpected(resp: Response) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("fude serve の応答が要求と合わない: {resp:?}"))
}

impl ProjectFs for RemoteFs {
    fn list_dir(&self, rel: &RelPath) -> io::Result<Vec<DirEntry>> {
        match self.call(Request::ListDir { root: self.root.clone(), path: rel.clone() }, &[])? {
            (Response::Entries { entries }, _) => Ok(entries),
            (resp, _) => Err(unexpected(resp)),
        }
    }

    fn read(&self, rel: &RelPath) -> io::Result<Vec<u8>> {
        match self.call(Request::Read { root: self.root.clone(), path: rel.clone() }, &[])? {
            (Response::Data, data) => Ok(data),
            (resp, _) => Err(unexpected(resp)),
        }
    }

    fn write(&self, rel: &RelPath, data: &[u8]) -> io::Result<()> {
        match self.call(Request::Write { root: self.root.clone(), path: rel.clone() }, data)? {
            (Response::Done, _) => Ok(()),
            (resp, _) => Err(unexpected(resp)),
        }
    }

    fn branch(&self) -> io::Result<Option<String>> {
        match self.call(Request::Branch { root: self.root.clone() }, &[])? {
            (Response::Branch { name }, _) => Ok(name),
            (resp, _) => Err(unexpected(resp)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, PipeReader, PipeWriter};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    type Pair = (Box<dyn Write + Send>, Box<dyn BufRead + Send>);

    /// サーバーをスレッドで動かし、パイプでつなぐ。
    fn pipes(server: impl FnOnce(BufReader<PipeReader>, PipeWriter) + Send + 'static) -> io::Result<Pair> {
        let (req_r, req_w) = std::io::pipe()?;
        let (resp_r, resp_w) = std::io::pipe()?;
        std::thread::spawn(move || server(BufReader::new(req_r), resp_w));
        Ok((Box::new(req_w), Box::new(BufReader::new(resp_r))))
    }

    struct ServeConnector;
    impl Connector for ServeConnector {
        fn connect(&self) -> io::Result<Pair> {
            pipes(|r, w| {
                let _ = crate::serve::serve(r, w, "test-id");
            })
        }
    }

    fn client(connector: impl Connector + 'static, timeout: Duration) -> Arc<RemoteClient> {
        Arc::new(RemoteClient::new(Box::new(connector), timeout))
    }

    fn rel(p: &str) -> RelPath {
        RelPath::new(p).unwrap()
    }

    #[test]
    fn remote_fs_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git").join("HEAD"), "ref: refs/heads/dev\n").unwrap();
        let fs = RemoteFs::new(client(ServeConnector, Duration::from_secs(5)), dir.path().to_string_lossy().into());
        let data = vec![0u8, b'\n', 255];
        fs.write(&rel("a/b.bin"), &data).unwrap();
        assert_eq!(fs.read(&rel("a/b.bin")).unwrap(), data);
        assert_eq!(fs.list_dir(&RelPath::root()).unwrap(), vec![DirEntry { name: "a".into(), is_dir: true }]);
        assert_eq!(fs.branch().unwrap(), Some("dev".into()));
    }

    #[test]
    fn remote_maps_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let fs = RemoteFs::new(client(ServeConnector, Duration::from_secs(5)), dir.path().to_string_lossy().into());
        assert_eq!(fs.read(&rel("none")).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    /// 受け取るが、何も返さない。
    struct SilentConnector(Mutex<Vec<PipeWriter>>);
    impl Connector for SilentConnector {
        fn connect(&self) -> io::Result<Pair> {
            let (r, w) = std::io::pipe()?;
            self.0.lock().unwrap().push(w);
            Ok((Box::new(io::sink()), Box::new(BufReader::new(r))))
        }
    }

    #[test]
    fn request_times_out() {
        let c = client(SilentConnector(Mutex::new(Vec::new())), Duration::from_millis(200));
        let start = Instant::now();
        let err = c.call(&Request::Version, &[]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    /// 1 回目の接続は 1 つの要求に答えて終わる。2 回目からは通常のサーバー。
    struct FlakyConnector(AtomicUsize);
    impl Connector for FlakyConnector {
        fn connect(&self) -> io::Result<Pair> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                pipes(|mut r, mut w| {
                    let _ = read_frame::<Request>(&mut r);
                    let _ = write_frame(&mut w, &Response::Version { id: "first".into() }, &[]);
                })
            } else {
                ServeConnector.connect()
            }
        }
    }

    struct Shared<C>(Arc<C>);
    impl<C: Connector> Connector for Shared<C> {
        fn connect(&self) -> io::Result<Pair> {
            self.0.connect()
        }
    }

    /// 使っていた接続が切れていたら（WSL の再起動など）、1 回だけつなぎ直して送り直す。
    #[test]
    fn reconnects_once_after_server_exit() {
        let connector = Arc::new(FlakyConnector(AtomicUsize::new(0)));
        let c = client(Shared(connector.clone()), Duration::from_secs(5));
        assert_eq!(c.call(&Request::Version, &[]).unwrap().0, Response::Version { id: "first".into() });
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(c.call(&Request::Version, &[]).unwrap().0, Response::Version { id: "test-id".into() });
        assert_eq!(connector.0.load(Ordering::SeqCst), 2);
    }

    /// つないだばかりの接続で失敗したら、送り直さない（起動できないサーバーを何度も起こさない）。
    struct DeadConnector(AtomicUsize);
    impl Connector for DeadConnector {
        fn connect(&self) -> io::Result<Pair> {
            self.0.fetch_add(1, Ordering::SeqCst);
            pipes(|_, _| {})
        }
    }

    #[test]
    fn fresh_connection_failure_is_not_retried() {
        let connector = Arc::new(DeadConnector(AtomicUsize::new(0)));
        let c = client(Shared(connector.clone()), Duration::from_secs(5));
        assert!(c.call(&Request::Version, &[]).is_err());
        assert_eq!(connector.0.load(Ordering::SeqCst), 1);
        assert!(c.call(&Request::Version, &[]).is_err());
        assert_eq!(connector.0.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn first_request_uses_first_timeout() {
        let c = RemoteClient::new(Box::new(SilentConnector(Mutex::new(Vec::new()))), Duration::from_millis(100))
            .with_first_timeout(Duration::from_millis(400));
        let start = Instant::now();
        assert!(c.call(&Request::Version, &[]).is_err());
        assert!(start.elapsed() >= Duration::from_millis(350));
    }
}
