//! fude serve のサーバーのループ。標準入出力で要求を受け、LocalFs で処理する。

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::wire::{Request, Response, read_frame, write_frame};
use crate::{LocalFs, ProjectFs};

/// 入力が終わるまで、要求ごとに LocalFs::new(root) で処理して応答を書く。root は絶対パス（WSL 側）。
/// 1 つの要求の失敗では止まらず、Error を返して続ける。読めないフレームを受けたら Error を返して終わる
/// （payload の長さがわからず、次のフレームの境目を見失うため）。
pub fn serve(mut input: impl BufRead, mut output: impl Write, build_id: &str) -> io::Result<()> {
    loop {
        match read_frame::<Request>(&mut input) {
            Ok(None) => return Ok(()),
            Ok(Some((req, payload))) => {
                let (resp, data) = handle(req, &payload, build_id);
                write_frame(&mut output, &resp, &data)?;
            }
            Err(e) => {
                let resp = Response::Error { message: format!("要求を読めない: {e}"), not_found: false, already_exists: false };
                write_frame(&mut output, &resp, &[])?;
                return Err(e);
            }
        }
    }
}

fn handle(req: Request, payload: &[u8], build_id: &str) -> (Response, Vec<u8>) {
    let result = match req {
        Request::Version => Ok((Response::Version { id: build_id.to_string() }, Vec::new())),
        Request::ListDir { root, path } => {
            local(&root).and_then(|fs| fs.list_dir(&path)).and_then(|entries| {
            let json = serde_json::to_vec(&entries).map_err(io::Error::other)?;
            Ok((Response::Entries, json))
        })
        }
        Request::Read { root, path } => local(&root).and_then(|fs| fs.read(&path)).map(|data| (Response::Data, data)),
        Request::Write { root, path } => {
            local(&root).and_then(|fs| fs.write(&path, payload)).map(|()| (Response::Done, Vec::new()))
        }
        Request::Branch { root } => {
            local(&root).and_then(|fs| fs.branch()).map(|name| (Response::Branch { name }, Vec::new()))
        }
        Request::WalkFiles { root } => local(&root).and_then(|fs| fs.walk_files()).and_then(|paths| {
            let json = serde_json::to_vec(&paths).map_err(io::Error::other)?;
            Ok((Response::Paths, json))
        }),
        Request::CreateFile { root, path } => done(local(&root).and_then(|fs| fs.create_file(&path))),
        Request::CreateDir { root, path } => done(local(&root).and_then(|fs| fs.create_dir(&path))),
        Request::Rename { root, from, to } => done(local(&root).and_then(|fs| fs.rename(&from, &to))),
        Request::Remove { root, path } => done(local(&root).and_then(|fs| fs.remove(&path))),
        Request::Metadata { root, path } => {
            local(&root).and_then(|fs| fs.metadata(&path)).map(|meta| (Response::Meta { meta }, Vec::new()))
        }
        Request::FindRoot { file } => find_root(&file).map(|path| (Response::Root { path }, Vec::new())),
        Request::GitStatus { root } => local(&root).and_then(|fs| fs.git_status()).and_then(|status| {
            let json = serde_json::to_vec(&status).map_err(io::Error::other)?;
            Ok((Response::Status, json))
        }),
        Request::GitDiff { root, path } => {
            local(&root).and_then(|fs| fs.git_diff(&path)).map(|patch| (Response::Patch, patch.into_bytes()))
        }
        Request::Busy { terminal_id } => crate::procs::terminal_busy(Path::new("/proc"), &terminal_id)
            .map(|busy| (Response::Busy { busy }, Vec::new())),
    };
    result.unwrap_or_else(|e| {
        let resp = Response::Error {
            message: e.to_string(),
            not_found: e.kind() == io::ErrorKind::NotFound,
            already_exists: e.kind() == io::ErrorKind::AlreadyExists,
        };
        (resp, Vec::new())
    })
}

fn done(result: io::Result<()>) -> io::Result<(Response, Vec<u8>)> {
    result.map(|()| (Response::Done, Vec::new()))
}

/// file は絶対パスに限る。`.git` を持つ最初の祖先、無ければファイルの親。
fn find_root(file: &str) -> io::Result<String> {
    let file = Path::new(file);
    if !file.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("絶対パスでない: {}", file.display())));
    }
    let root = fude_core::location::find_project_root(file, &|d| d.join(".git").exists());
    Ok(root.to_string_lossy().into_owned())
}

/// root は絶対パスに限る（相対パスでは、サーバーの作業ディレクトリ次第で場所が変わるため）。
fn local(root: &str) -> io::Result<LocalFs> {
    if !Path::new(root).is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("ルートが絶対パスでない: {root}")));
    }
    Ok(LocalFs::new(PathBuf::from(root)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RelPath;
    use crate::wire::{read_frame, write_frame};
    use std::io::Cursor;

    fn run(requests: &[(Request, Vec<u8>)]) -> Vec<(Response, Vec<u8>)> {
        let mut input = Vec::new();
        for (req, payload) in requests {
            write_frame(&mut input, req, payload).unwrap();
        }
        responses(&input)
    }

    fn responses(input: &[u8]) -> Vec<(Response, Vec<u8>)> {
        let mut output = Vec::new();
        let _ = serve(Cursor::new(input.to_vec()), &mut output, "0.1.0+1");
        let mut r = Cursor::new(output);
        let mut out = Vec::new();
        while let Some(frame) = read_frame::<Response>(&mut r).unwrap() {
            out.push(frame);
        }
        out
    }

    fn root_of(dir: &tempfile::TempDir) -> String {
        dir.path().to_string_lossy().into_owned()
    }

    #[test]
    fn serve_lists_reads_writes_and_branch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let root = root_of(&dir);
        let path = RelPath::new("a.bin").unwrap();
        let data = vec![0u8, 10, 13, 255, b'\n'];
        let out = run(&[
            (Request::Version, vec![]),
            (Request::Write { root: root.clone(), path: path.clone() }, data.clone()),
            (Request::Read { root: root.clone(), path: path.clone() }, vec![]),
            (Request::ListDir { root: root.clone(), path: RelPath::root() }, vec![]),
            (Request::Branch { root: root.clone() }, vec![]),
        ]);
        assert_eq!(out.len(), 5);
        assert_eq!(out[0].0, Response::Version { id: "0.1.0+1".into() });
        assert_eq!(out[1].0, Response::Done);
        assert_eq!(out[2], (Response::Data, data));
        assert_eq!(out[3].0, Response::Entries);
        let entries: Vec<crate::DirEntry> = serde_json::from_slice(&out[3].1).unwrap();
        assert_eq!(entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["a.bin"]);
        assert_eq!(out[4].0, Response::Branch { name: Some("main".into()) });
    }

    #[test]
    fn serve_rejects_escape() {
        let dir = tempfile::tempdir().unwrap();
        let header = serde_json::json!({"op": "read", "root": root_of(&dir), "path": "../x", "len": 0});
        let input = format!("{header}\n");
        let out = responses(input.as_bytes());
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0].0, Response::Error { not_found: false, .. }), "{:?}", out[0]);
    }

    #[test]
    fn serve_reports_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let out = run(&[
            (Request::Read { root: root_of(&dir), path: RelPath::new("none").unwrap() }, vec![]),
            (Request::Version, vec![]),
        ]);
        assert!(matches!(out[0].0, Response::Error { not_found: true, .. }), "{:?}", out[0]);
        assert_eq!(out[1].0, Response::Version { id: "0.1.0+1".into() });
    }

    #[test]
    fn serve_rejects_relative_root() {
        let out = run(&[(Request::Branch { root: "home/x".into() }, vec![])]);
        assert!(matches!(out[0].0, Response::Error { not_found: false, .. }), "{:?}", out[0]);
    }
}
