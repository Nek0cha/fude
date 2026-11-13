//! WSL の fude serve を wsl.exe の子プロセスとして起動する Connector と、Linux 用の fude の配置（Windows だけ）。
//!
//! WSL 側のパスは "~/" で始めてよい（シェルの中で $HOME に直す）。wsl.exe には引数を `-e` で渡し、
//! シェルの解釈を通さない（検証4の申し送り1）。

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::remote::Connector;

/// コンソールの窓を出さない。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 置き場所の既定。
pub const DEFAULT_BIN: &str = "~/.local/bin/fude";
/// 既定の置き場所のときに、fude.exe の場所を書くファイル。
const DEFAULT_HOST_EXE_FILE: &str = "~/.config/fude/host-exe";

/// "~/" で始まる引数を $HOME の下に直す関数 r を定義するシェルの前置き。
const RESOLVE: &str = r#"r() { case "$1" in '~/'*) printf '%s/%s' "$HOME" "${1#??}";; *) printf '%s' "$1";; esac; }; "#;

/// 置いてある fude が無い（実行できない）ときの終了コード。
const NOT_INSTALLED: i32 = 3;

/// `FUDE_WSL_BIN_DIR` があれば "<dir>/fude"（開発中の試験用）、無ければ DEFAULT_BIN。
pub fn default_bin(env: &dyn Fn(&str) -> Option<String>) -> String {
    match env("FUDE_WSL_BIN_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => format!("{}/fude", dir.trim_end_matches('/')),
        None => DEFAULT_BIN.to_string(),
    }
}

/// fude.exe の場所を書くファイル。既定の置き場所なら ~/.config/fude/host-exe、それ以外は bin と同じ
/// ディレクトリの host-exe（試験でユーザーの設定を書き換えないため。Linux 用の fude は自分の隣を先に見る）。
fn host_exe_file(bin: &str) -> String {
    if bin == DEFAULT_BIN {
        return DEFAULT_HOST_EXE_FILE.to_string();
    }
    match bin.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/host-exe"),
        None => "host-exe".to_string(),
    }
}

fn wsl_command(distro: &str, script: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new("wsl.exe");
    cmd.args(["-d", distro, "-e", "sh", "-c", script, "sh"])
        .args(args)
        .env("WSL_UTF8", "1")
        .creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// wsl.exe の出力を文字列にする。NUL を含めば UTF-16LE として読む（WSL_UTF8 が効かない版のため）。
fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.contains(&0) {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// 一覧の取得を待つ時間。VM を起こさないため短い。
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
/// 版の確認を待つ時間。止まっている WSL の起動を含む。
const VERSION_TIMEOUT: Duration = Duration::from_secs(30);
/// 配置を待つ時間。
const DEPLOY_TIMEOUT: Duration = Duration::from_secs(60);

/// cmd を起動し、stdin を流して、終わるまで待つ。timeout を過ぎたら子プロセスを終わらせて Err(TimedOut)。
/// スリープ復帰のあとなどに wsl.exe が固まっても、呼び出し側が待ち続けないようにする（検証4の申し送り5）。
fn output_with_timeout(mut cmd: Command, stdin: &[u8], timeout: Duration) -> io::Result<Output> {
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut input = child.stdin.take().expect("stdin はパイプにしてある");
    let data = stdin.to_vec();
    let writer = std::thread::spawn(move || input.write_all(&data));
    let mut out_pipe = child.stdout.take().expect("stdout はパイプにしてある");
    let mut err_pipe = child.stderr.take().expect("stderr はパイプにしてある");
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("wsl.exe が {} 秒以内に終わらない", timeout.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let written = writer.join().unwrap_or_else(|_| Err(io::Error::other("stdin へ書けない")));
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    if let Err(e) = written
        && status.success()
    {
        return Err(e);
    }
    Ok(Output { status, stdout, stderr })
}

/// distro がインストールされていなければ Err(NotFound)。名前の大文字小文字は区別しない（wsl.exe と同じ）。
fn ensure_distro(distro: &str) -> io::Result<()> {
    let mut cmd = Command::new("wsl.exe");
    cmd.args(["-l", "-q"]).env("WSL_UTF8", "1").creation_flags(CREATE_NO_WINDOW);
    let out = output_with_timeout(cmd, &[], LIST_TIMEOUT)?;
    let text = decode_wsl_output(&out.stdout);
    let installed: Vec<&str> = text
        .lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0'))
        .filter(|l| !l.is_empty())
        .collect();
    if installed.iter().any(|d| d.eq_ignore_ascii_case(distro)) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("WSL のディストリビューション '{distro}' が無い（インストール済み: {}）", installed.join(", ")),
    ))
}

/// 置いてある fude の版。無い、起動できない → Ok(None)。ディストリビューションが無い → Err(NotFound)。
pub fn installed_version(distro: &str, bin: &str) -> io::Result<Option<String>> {
    ensure_distro(distro)?;
    let script = format!(r#"{RESOLVE}b=$(r "$1"); [ -x "$b" ] || exit {NOT_INSTALLED}; exec "$b" version"#);
    let out = output_with_timeout(wsl_command(distro, &script, &[bin]), &[], VERSION_TIMEOUT)?;
    if !out.status.success() {
        return Ok(None);
    }
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!id.is_empty()).then_some(id))
}

/// bytes を bin に置く（標準入力で流し、一時ファイルに書いてから chmod +x と mv）。
/// あわせて host_exe（Windows のパス）を `wslpath -u` で直したものを host-exe のファイルに書く。
pub fn deploy(distro: &str, bin: &str, bytes: &[u8], host_exe: &Path) -> io::Result<()> {
    let host_exe = host_exe.to_str().ok_or_else(|| io::Error::other("fude.exe のパスを読めない"))?;
    let script = format!(
        r#"set -e; {RESOLVE}b=$(r "$1"); h=$(r "$3"); mkdir -p "$(dirname "$b")" "$(dirname "$h")"; t="$b.tmp.$$"; cat > "$t"; chmod +x "$t"; mv -f "$t" "$b"; wslpath -u "$2" > "$h.tmp.$$"; mv -f "$h.tmp.$$" "$h""#
    );
    let cmd = wsl_command(distro, &script, &[bin, host_exe, &host_exe_file(bin)]);
    let out = output_with_timeout(cmd, bytes, DEPLOY_TIMEOUT)?;
    if !out.status.success() {
        let detail = decode_wsl_output(&out.stderr);
        return Err(io::Error::other(format!("WSL に fude を置けない（{}）: {}", out.status, detail.trim())));
    }
    Ok(())
}

/// `wsl.exe -d <distro> -e sh -c '… exec "$b" serve' sh <bin>` を起動する Connector。
pub struct WslConnector {
    pub distro: String,
    pub bin: String,
}

impl Connector for WslConnector {
    fn connect(&self) -> io::Result<(Box<dyn Write + Send>, Box<dyn BufRead + Send>)> {
        let script = format!(r#"{RESOLVE}b=$(r "$1"); exec "$b" serve"#);
        let mut child = wsl_command(&self.distro, &script, &[&self.bin])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin はパイプにしてある");
        let stdout = child.stdout.take().expect("stdout はパイプにしてある");
        let mut stderr = child.stderr.take().expect("stderr はパイプにしてある");
        let errors = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&errors);
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            *sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(decode_wsl_output(&buf));
        });
        let reader = ServerOutput { stdout, errors };
        Ok((Box::new(ServerInput { stdin, child }), Box::new(BufReader::new(reader))))
    }
}

/// サーバーの標準入力。捨てたらサーバーを終わらせる。
struct ServerInput {
    stdin: ChildStdin,
    child: Child,
}

impl Write for ServerInput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdin.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stdin.flush()
    }
}

impl Drop for ServerInput {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// サーバーの標準出力。終わったら、標準エラーの中身を添えたエラーにする（検証4の申し送り2）。
struct ServerOutput {
    stdout: ChildStdout,
    errors: Arc<Mutex<Option<String>>>,
}

impl Read for ServerOutput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.stdout.read(buf)?;
        if n > 0 || buf.is_empty() {
            return Ok(n);
        }
        // 標準エラーを読み終えるのを少しだけ待つ。
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if let Some(text) = self.errors.lock().unwrap_or_else(|e| e.into_inner()).as_deref() {
                let text = text.trim();
                if text.is_empty() {
                    return Ok(0);
                }
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, format!("fude serve が終わった: {text}")));
            }
            if Instant::now() >= deadline {
                return Ok(0);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// WSL のフォルダの監視。`fude watch <root>` を wsl.exe の子プロセスとして動かし、1 行ずつ WatchBatch を読む。
/// 子プロセスが終わったら、crate::watch::backoff の間をおいて起動し直す。捨てると子プロセスを終わらせる。
/// Linux 用の fude が置いてあること（WslHosts の確認と配置のあと）を前提にする。
pub struct WslWatcher {
    stop: Arc<std::sync::atomic::AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl WslWatcher {
    pub fn start(distro: String, bin: String, root: String, on_batch: Box<dyn Fn(crate::watch::WatchBatch) + Send>) -> WslWatcher {
        use std::sync::atomic::Ordering;
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let (stop2, child2) = (stop.clone(), child.clone());
        std::thread::spawn(move || {
            let mut attempt = 0u32;
            while !stop2.load(Ordering::Relaxed) {
                let script = format!(r#"{RESOLVE}b=$(r "$1"); exec "$b" watch "$2""#);
                let spawned = wsl_command(&distro, &script, &[&bin, &root])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
                let mut received = false;
                if let Ok(mut c) = spawned {
                    let stdout = c.stdout.take().expect("stdout はパイプにしてある");
                    *child2.lock().unwrap_or_else(|e| e.into_inner()) = Some(c);
                    for line in BufReader::new(stdout).lines() {
                        let Ok(line) = line else { break };
                        if let Ok(batch) = serde_json::from_str::<crate::watch::WatchBatch>(&line) {
                            received = true;
                            on_batch(batch);
                        }
                    }
                    if let Some(mut c) = child2.lock().unwrap_or_else(|e| e.into_inner()).take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                }
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                // 通知を受け取れていた接続のあとは、待ち時間を最初から数え直す。
                attempt = if received { 0 } else { attempt + 1 };
                std::thread::sleep(crate::watch::backoff(attempt));
            }
        });
        WslWatcher { stop, child }
    }
}

impl Drop for WslWatcher {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(mut c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hung_child_is_killed_after_timeout() {
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "6", "127.0.0.1"]);
        let start = Instant::now();
        let err = output_with_timeout(cmd, &[], Duration::from_millis(300)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn child_output_and_stdin_are_collected() {
        let mut cmd = Command::new("findstr");
        cmd.arg("x");
        let out = output_with_timeout(cmd, b"axb\r\nc\r\n", Duration::from_secs(5)).unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "axb");
    }

    #[test]
    fn host_exe_file_rules() {
        assert_eq!(host_exe_file(DEFAULT_BIN), "~/.config/fude/host-exe");
        assert_eq!(host_exe_file("/tmp/x/fude"), "/tmp/x/host-exe");
    }
}
