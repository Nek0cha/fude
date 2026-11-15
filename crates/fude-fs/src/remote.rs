//! fude serve と話すクライアント。OS に依らず、任意の読み書きの組（子プロセスの標準入出力など）で動く。

use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::wire::{Request, Response, read_frame, write_frame};
use crate::{DirEntry, FileMeta, ProjectFs, RelPath};

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
    heavy: std::sync::RwLock<crate::ignored::HeavyDirs>,
}

impl RemoteFs {
    pub fn new(client: Arc<RemoteClient>, root: String) -> RemoteFs {
        RemoteFs { client, root, heavy: std::sync::RwLock::new(Default::default()) }
    }

    fn heavy(&self) -> crate::ignored::HeavyDirs {
        self.heavy.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn call(&self, req: Request, payload: &[u8]) -> io::Result<(Response, Vec<u8>)> {
        call_checked(&self.client, req, payload)
    }

    fn done(&self, req: Request) -> io::Result<()> {
        match self.call(req, &[])? {
            (Response::Done, _) => Ok(()),
            (resp, _) => Err(unexpected(resp)),
        }
    }
}

/// Response::Error を io::Error にして返す。
fn call_checked(client: &RemoteClient, req: Request, payload: &[u8]) -> io::Result<(Response, Vec<u8>)> {
    match client.call(&req, payload)? {
        (Response::Error { message, not_found, already_exists, invalid_input }, _) => {
            let kind = if not_found {
                io::ErrorKind::NotFound
            } else if already_exists {
                io::ErrorKind::AlreadyExists
            } else if invalid_input {
                io::ErrorKind::InvalidInput
            } else {
                io::ErrorKind::Other
            };
            Err(io::Error::new(kind, message))
        }
        frame => Ok(frame),
    }
}

/// サーバー側でファイル（絶対パス）からプロジェクトのルートを探す。
pub fn find_root(client: &RemoteClient, file: &str) -> io::Result<String> {
    match call_checked(client, Request::FindRoot { file: file.to_string() }, &[])? {
        (Response::Root { path }, _) => Ok(path),
        (resp, _) => Err(unexpected(resp)),
    }
}

/// サーバー側で、terminal_id のシェルの上で何かが動いているかを調べる。
pub fn busy(client: &RemoteClient, terminal_id: &str) -> io::Result<bool> {
    match call_checked(client, Request::Busy { terminal_id: terminal_id.to_string() }, &[])? {
        (Response::Busy { busy }, _) => Ok(busy),
        (resp, _) => Err(unexpected(resp)),
    }
}

fn unexpected(resp: Response) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("fude serve の応答が要求と合わない: {resp:?}"))
}

impl ProjectFs for RemoteFs {
    fn set_heavy_dirs(&self, heavy: crate::ignored::HeavyDirs) {
        *self.heavy.write().unwrap_or_else(|e| e.into_inner()) = heavy;
    }

    fn list_dir(&self, rel: &RelPath) -> io::Result<Vec<DirEntry>> {
        match self.call(Request::ListDir { root: self.root.clone(), path: rel.clone(), heavy: self.heavy() }, &[])? {
            (Response::Entries, json) => serde_json::from_slice(&json)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("一覧を読めない: {e}"))),
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

    fn walk_files(&self) -> io::Result<crate::WalkedFiles> {
        match self.call(Request::WalkFiles { root: self.root.clone(), heavy: self.heavy() }, &[])? {
            (Response::Paths, json) => serde_json::from_slice(&json)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("一覧を読めない: {e}"))),
            (resp, _) => Err(unexpected(resp)),
        }
    }

    fn create_file(&self, rel: &RelPath) -> io::Result<()> {
        self.done(Request::CreateFile { root: self.root.clone(), path: rel.clone() })
    }

    fn create_dir(&self, rel: &RelPath) -> io::Result<()> {
        self.done(Request::CreateDir { root: self.root.clone(), path: rel.clone() })
    }

    fn rename(&self, from: &RelPath, to: &RelPath) -> io::Result<()> {
        self.done(Request::Rename { root: self.root.clone(), from: from.clone(), to: to.clone() })
    }

    fn remove(&self, rel: &RelPath) -> io::Result<()> {
        self.done(Request::Remove { root: self.root.clone(), path: rel.clone() })
    }

    fn trash(&self, rel: &RelPath) -> io::Result<crate::TrashTicket> {
        match self.call(Request::Trash { root: self.root.clone(), path: rel.clone() }, &[])? {
            (Response::Trashed { ticket }, _) => Ok(ticket),
            (resp, _) => Err(unexpected(resp)),
        }
    }

    fn restore(&self, ticket: &crate::TrashTicket, rel: &RelPath) -> io::Result<()> {
        self.done(Request::Restore { root: self.root.clone(), ticket: ticket.clone(), path: rel.clone() })
    }

    fn git_status(&self) -> io::Result<Option<crate::git::GitStatus>> {
        match self.call(Request::GitStatus { root: self.root.clone() }, &[])? {
            (Response::Status, json) => serde_json::from_slice(&json)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Git の状態を読めない: {e}"))),
            (resp, _) => Err(unexpected(resp)),
        }
    }

    fn git_diff(&self, rel: &RelPath) -> io::Result<String> {
        match self.call(Request::GitDiff { root: self.root.clone(), path: rel.clone() }, &[])? {
            (Response::Patch, data) => Ok(String::from_utf8_lossy(&data).into_owned()),
            (resp, _) => Err(unexpected(resp)),
        }
    }

    fn plan_files(&self, known: Option<&crate::plans::PlanStamp>) -> io::Result<crate::plans::PlanFetch> {
        match self.call(Request::PlanFiles { root: self.root.clone(), known: known.cloned() }, &[])? {
            (Response::Plan, json) => serde_json::from_slice(&json)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("プランを読めない: {e}"))),
            (resp, _) => Err(unexpected(resp)),
        }
    }

    fn search(&self, query: &crate::search::SearchQuery) -> io::Result<crate::search::SearchResults> {
        match self.call(Request::Search { root: self.root.clone(), query: query.clone() }, &[])? {
            (Response::Matches, json) => serde_json::from_slice(&json)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("検索の結果を読めない: {e}"))),
            (resp, _) => Err(unexpected(resp)),
        }
    }

    fn metadata(&self, rel: &RelPath) -> io::Result<FileMeta> {
        match self.call(Request::Metadata { root: self.root.clone(), path: rel.clone() }, &[])? {
            (Response::Meta { meta }, _) => Ok(meta),
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
        assert_eq!(fs.list_dir(&RelPath::root()).unwrap(), vec![DirEntry { name: "a".into(), is_dir: true, ignored: false }]);
        assert_eq!(fs.branch().unwrap(), Some("dev".into()));
    }

    /// 一覧がヘッダーの上限（64 KiB）を超える大きさでも届く。
    #[test]
    fn large_dir_listing_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5000 {
            std::fs::write(dir.path().join(format!("component-{i:05}.tsx")), "").unwrap();
        }
        let fs = RemoteFs::new(client(ServeConnector, Duration::from_secs(10)), dir.path().to_string_lossy().into());
        let entries = fs.list_dir(&RelPath::root()).unwrap();
        assert_eq!(entries.len(), 5000);
        assert_eq!(entries[4999].name, "component-04999.tsx");
    }

    #[test]
    fn remote_trash_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let fs = RemoteFs::new(client(ServeConnector, Duration::from_secs(10)), dir.path().to_string_lossy().into());
        std::fs::write(dir.path().join("r.txt"), "one").unwrap();
        let ticket = fs.trash(&rel("r.txt")).unwrap();
        assert!(!dir.path().join("r.txt").exists());
        std::fs::write(dir.path().join("r.txt"), "new").unwrap();
        assert_eq!(fs.restore(&ticket, &rel("r.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        std::fs::remove_file(dir.path().join("r.txt")).unwrap();
        fs.restore(&ticket, &rel("r.txt")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("r.txt")).unwrap(), "one");
        assert_eq!(fs.trash(&rel("none")).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn remote_maps_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let fs = RemoteFs::new(client(ServeConnector, Duration::from_secs(5)), dir.path().to_string_lossy().into());
        assert_eq!(fs.read(&rel("none")).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn remote_search_maps_invalid_input() {
        use crate::search::SearchQuery;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "foo\n").unwrap();
        let fs = RemoteFs::new(client(ServeConnector, Duration::from_secs(5)), dir.path().to_string_lossy().into());
        assert_eq!(fs.search(&SearchQuery { pattern: "foo".into(), ..Default::default() }).unwrap().files.len(), 1);
        let err = fs.search(&SearchQuery { pattern: "(".into(), regex: true, ..Default::default() }).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
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

    #[test]
    fn remote_git_status_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let fs = RemoteFs::new(client(ServeConnector, Duration::from_secs(5)), dir.path().to_string_lossy().into());
        assert_eq!(fs.git_status().unwrap(), None);
        let git = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(dir.path()).output().is_ok_and(|o| o.status.success())
        };
        if !git(&["init", "-q"]) {
            return;
        }
        std::fs::write(dir.path().join("a.txt"), "1\n").unwrap();
        let status = fs.git_status().unwrap().unwrap();
        assert_eq!(status.mark(&rel("a.txt")), Some(crate::git::GitMark::Untracked));
        assert_eq!(fs.git_diff(&rel("a.txt")).unwrap(), "");
    }

    #[test]
    fn remote_fs_new_ops_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo").join(".git")).unwrap();
        let c = client(ServeConnector, Duration::from_secs(5));
        let fs = RemoteFs::new(c.clone(), dir.path().to_string_lossy().into());
        fs.create_dir(&rel("repo/src")).unwrap();
        fs.create_file(&rel("repo/src/a.rs")).unwrap();
        assert_eq!(fs.create_file(&rel("repo/src/a.rs")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        fs.rename(&rel("repo/src/a.rs"), &rel("repo/src/b.rs")).unwrap();
        assert_eq!(fs.walk_files().unwrap().tracked, vec![rel("repo/src/b.rs")]);
        assert!(!fs.metadata(&rel("repo/src/b.rs")).unwrap().is_dir);
        let file = dir.path().join("repo").join("src").join("b.rs");
        let root = find_root(&c, &file.to_string_lossy()).unwrap();
        assert_eq!(std::path::PathBuf::from(root), dir.path().join("repo"));
        assert!(find_root(&c, "relative/x").is_err());
        fs.remove(&rel("repo/src")).unwrap();
        assert!(fs.walk_files().unwrap().tracked.is_empty());
    }
}
