//! fude serve の通信の形式。1 つのフレーム = JSON 1 行（改行で終わる）＋ 続けて len バイトの生のデータ。
//! ファイルの中身を JSON に入れないため（大きさ、NUL、改行）。len はヘッダーの JSON に足して送る。

use std::io::{self, BufRead, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{DirEntry, RelPath};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Version,
    ListDir { root: String, path: RelPath },
    Read { root: String, path: RelPath },
    /// 中身は payload。
    Write { root: String, path: RelPath },
    Branch { root: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Version { id: String },
    Entries { entries: Vec<DirEntry> },
    /// 中身は payload。
    Data,
    Done,
    Branch { name: Option<String> },
    Error { message: String, not_found: bool },
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
