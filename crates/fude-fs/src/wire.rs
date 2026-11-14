//! fude serve の通信の形式。1 つのフレーム = JSON 1 行（改行で終わる）＋ 続けて len バイトの生のデータ。
//! ファイルの中身を JSON に入れないため（大きさ、NUL、改行）。len はヘッダーの JSON に足して送る。

use std::io::{self, BufRead, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::RelPath;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Version,
    ListDir { root: String, path: RelPath },
    Read { root: String, path: RelPath },
    /// 中身は payload。
    Write { root: String, path: RelPath },
    Branch { root: String },
    WalkFiles { root: String },
    CreateFile { root: String, path: RelPath },
    CreateDir { root: String, path: RelPath },
    Rename { root: String, from: RelPath, to: RelPath },
    Remove { root: String, path: RelPath },
    Metadata { root: String, path: RelPath },
    /// ファイル（Linux の絶対パス）からプロジェクトのルートを探す（fude_core::location::find_project_root）。
    FindRoot { file: String },
    /// FUDE_TERMINAL_ID が terminal_id のシェルの上で、何かが動いているか。
    Busy { terminal_id: String },
    GitStatus { root: String },
    GitDiff { root: String, path: RelPath },
    /// 全体検索（計画6で足した）。
    Search { root: String, query: crate::search::SearchQuery },
    /// 最も新しいプランと台帳（計画7で足した）。
    PlanFiles { root: String, known: Option<crate::plans::PlanStamp> },
    /// ゴミ箱に送る（計画8bで足した）。
    Trash { root: String, path: RelPath },
    /// ゴミ箱から path へ戻す（計画8bで足した）。
    Restore { root: String, ticket: crate::TrashTicket, path: RelPath },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Version { id: String },
    /// 一覧（Vec<DirEntry> の JSON）は payload。大きいディレクトリでヘッダーの上限を超えないため。
    Entries,
    /// 中身は payload。
    Data,
    Done,
    Branch { name: Option<String> },
    /// 一覧（Vec<RelPath> の JSON）は payload。
    Paths,
    Meta { meta: crate::FileMeta },
    Root { path: String },
    Busy { busy: bool },
    /// Option<GitStatus> の JSON は payload。
    Status,
    /// unified diff は payload。
    Patch,
    /// SearchResults の JSON は payload。
    Matches,
    /// PlanFetch の JSON は payload。
    Plan,
    /// ゴミ箱に送った札。
    Trashed { ticket: crate::TrashTicket },
    /// already_exists は計画5で足した（古いサーバーの応答には無い）。
    Error {
        message: String,
        not_found: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        already_exists: bool,
        /// 計画6で足した（正しくない検索の条件）。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        invalid_input: bool,
    },
}

/// ヘッダーの 1 行の上限。
pub const MAX_HEADER: usize = 64 * 1024;
/// payload の上限。壊れた len で巨大な領域を取らないため。
pub const MAX_PAYLOAD: u64 = 1 << 30;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// header の JSON に "len" を足した 1 行と、payload を書いてから flush する。
pub fn write_frame<T: Serialize>(w: &mut impl Write, header: &T, payload: &[u8]) -> io::Result<()> {
    let mut value = serde_json::to_value(header).map_err(|e| invalid(e.to_string()))?;
    let serde_json::Value::Object(map) = &mut value else {
        return Err(invalid("ヘッダーが JSON のオブジェクトでない"));
    };
    map.insert("len".into(), payload.len().into());
    let mut line = serde_json::to_vec(&value).map_err(|e| invalid(e.to_string()))?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.write_all(payload)?;
    w.flush()
}

/// 入力が終わっていれば Ok(None)。ヘッダーが MAX_HEADER を超える、JSON でない、型に合わない → Err(InvalidData)。
/// ヘッダーや payload の途中で終わった → Err(UnexpectedEof)。
pub fn read_frame<T: DeserializeOwned>(r: &mut impl BufRead) -> io::Result<Option<(T, Vec<u8>)>> {
    let mut line = Vec::new();
    let n = r.take(MAX_HEADER as u64 + 1).read_until(b'\n', &mut line)?;
    if n == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        if line.len() > MAX_HEADER {
            return Err(invalid("ヘッダーが長すぎる"));
        }
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "ヘッダーの途中で終わった"));
    }
    let mut value: serde_json::Value = serde_json::from_slice(&line).map_err(|e| invalid(e.to_string()))?;
    let len = match &mut value {
        serde_json::Value::Object(map) => match map.remove("len") {
            None => 0,
            Some(v) => v.as_u64().ok_or_else(|| invalid("len が数でない"))?,
        },
        _ => return Err(invalid("ヘッダーが JSON のオブジェクトでない")),
    };
    if len > MAX_PAYLOAD {
        return Err(invalid("payload が大きすぎる"));
    }
    let header = serde_json::from_value(value).map_err(|e| invalid(e.to_string()))?;
    let mut payload = vec![0; len as usize];
    r.read_exact(&mut payload)?;
    Ok(Some((header, payload)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn binary_payload_roundtrip() {
        let req = Request::Write { root: "/home/x".into(), path: RelPath::new("a/b.bin").unwrap() };
        let payload: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
        assert!(payload.contains(&b'\n') && payload.contains(&0));
        let mut buf = Vec::new();
        write_frame(&mut buf, &req, &payload).unwrap();
        write_frame(&mut buf, &Request::Version, &[]).unwrap();
        let mut r = Cursor::new(buf);
        let (got, data) = read_frame::<Request>(&mut r).unwrap().unwrap();
        assert_eq!(got, req);
        assert_eq!(data, payload);
        let (got, data) = read_frame::<Request>(&mut r).unwrap().unwrap();
        assert_eq!(got, Request::Version);
        assert!(data.is_empty());
        assert!(read_frame::<Request>(&mut r).unwrap().is_none());
    }

    #[test]
    fn header_shape() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &Request::Version, &[]).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v, serde_json::json!({"op": "version", "len": 0}));
    }

    #[test]
    fn trash_request_round_trips() {
        let ticket = crate::TrashTicket { id: "/home/x/.local/share/Trash/info/a b.md.trashinfo".into() };
        let reqs = [
            Request::Trash { root: "/home/x".into(), path: RelPath::new("a b.md").unwrap() },
            Request::Restore { root: "/home/x".into(), ticket: ticket.clone(), path: RelPath::new("d/a.md").unwrap() },
        ];
        for req in reqs {
            let mut buf = Vec::new();
            write_frame(&mut buf, &req, &[]).unwrap();
            let (back, _) = read_frame::<Request>(&mut Cursor::new(buf)).unwrap().unwrap();
            assert_eq!(back, req);
        }
        let mut buf = Vec::new();
        write_frame(&mut buf, &Response::Trashed { ticket: ticket.clone() }, &[]).unwrap();
        let (back, _) = read_frame::<Response>(&mut Cursor::new(buf)).unwrap().unwrap();
        assert_eq!(back, Response::Trashed { ticket });
    }

    #[test]
    fn eof_is_none() {
        assert!(read_frame::<Request>(&mut Cursor::new(Vec::new())).unwrap().is_none());
    }

    #[test]
    fn oversized_header_is_error() {
        let mut r = Cursor::new(vec![b'a'; 70 * 1024]);
        let err = read_frame::<Request>(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn truncated_payload_is_error() {
        let mut r = Cursor::new(b"{\"op\":\"version\",\"len\":10}\nabc".to_vec());
        assert!(read_frame::<Request>(&mut r).is_err());
    }
}
