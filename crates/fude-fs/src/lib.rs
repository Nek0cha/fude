//! プロジェクトのファイルへのアクセスをまとめる。ローカルのフォルダも WSL のフォルダも、同じ ProjectFs で扱う。

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod local;
pub mod procs;
mod remote;
pub mod serve;
pub mod wire;
#[cfg(windows)]
pub mod wsl;

pub use local::LocalFs;
pub use remote::{Connector, RemoteClient, RemoteFs, busy, find_root};

/// プロジェクトのルートからの相対パス。"/" 区切り。空はルート自身。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelPath(String);

/// RelPath にできないパス（ルートの外を指す、など）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathError(String);

impl RelPath {
    /// "\" は "/" に直す。先頭と末尾の "/"、"." の要素は除く。".." の要素、ドライブ名（"C:"）、NUL を含むものは Err。
    pub fn new(path: &str) -> Result<RelPath, PathError> {
        if path.contains('\0') {
            return Err(PathError(format!("NUL を含む: {path:?}")));
        }
        let path = path.replace('\\', "/");
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
        if segments.contains(&"..") {
            return Err(PathError(format!("プロジェクトの外を指す: {path}")));
        }
        // Windows では、どの位置でも "C:…" の要素を push するとパス全体が置き換わるため、どこにあっても拒む。
        if segments.iter().any(|s| s.len() >= 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':') {
            return Err(PathError(format!("ドライブ名を含む: {path}")));
        }
        Ok(RelPath(segments.join("/")))
    }

    pub fn root() -> RelPath {
        RelPath(String::new())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn join(&self, name: &str) -> Result<RelPath, PathError> {
        RelPath::new(&format!("{}/{name}", self.0))
    }

    /// base の下の実際のパス。
    pub fn to_path(&self, base: &Path) -> PathBuf {
        let mut path = base.to_path_buf();
        for segment in self.0.split('/').filter(|s| !s.is_empty()) {
            path.push(segment);
        }
        path
    }
}

impl TryFrom<String> for RelPath {
    type Error = PathError;
    fn try_from(value: String) -> Result<RelPath, PathError> {
        RelPath::new(&value)
    }
}

impl From<RelPath> for String {
    fn from(value: RelPath) -> String {
        value.0
    }
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PathError {}

impl From<PathError> for io::Error {
    fn from(e: PathError) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, e.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// ファイルの大きさと更新日時。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    pub is_dir: bool,
    pub len: u64,
    /// 更新日時（UNIX ミリ秒）。取れなければ 0。
    pub modified_ms: u64,
}

/// walk_files で返す件数の上限。
pub const WALK_MAX: usize = 200_000;

pub trait ProjectFs: Send + Sync {
    /// rel の直下。gitignore を尊重し、隠しファイルは出し、.git は出さない。ディレクトリが先、各々名前順。
    fn list_dir(&self, rel: &RelPath) -> io::Result<Vec<DirEntry>>;
    fn read(&self, rel: &RelPath) -> io::Result<Vec<u8>>;
    /// 同じディレクトリの一時ファイルに書いてから rename する。
    fn write(&self, rel: &RelPath, data: &[u8]) -> io::Result<()>;
    /// ブランチ名（fude_core::git_head::read_branch と同じ規則）。
    fn branch(&self) -> io::Result<Option<String>>;
    /// gitignore を尊重した全ファイル（.git を除く）。名前順。WALK_MAX 件で打ち切る。
    fn walk_files(&self) -> io::Result<Vec<RelPath>>;
    /// 空のファイルを作る。あれば AlreadyExists。
    fn create_file(&self, rel: &RelPath) -> io::Result<()>;
    /// フォルダを作る（途中のフォルダも）。
    fn create_dir(&self, rel: &RelPath) -> io::Result<()>;
    /// 名前を変える。to があれば AlreadyExists。
    fn rename(&self, from: &RelPath, to: &RelPath) -> io::Result<()>;
    /// 消す。フォルダは中身ごと。ルートは拒む。
    fn remove(&self, rel: &RelPath) -> io::Result<()>;
    fn metadata(&self, rel: &RelPath) -> io::Result<FileMeta>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(p: &str) -> String {
        RelPath::new(p).unwrap().as_str().to_string()
    }

    #[test]
    fn rel_path_normalizes() {
        assert_eq!(rel("src\\a.rs"), "src/a.rs");
        assert_eq!(rel("/src/./a/"), "src/a");
        assert_eq!(RelPath::new("").unwrap(), RelPath::root());
        assert_eq!(RelPath::new(".").unwrap(), RelPath::root());
        assert_eq!(rel("作業/my proj/a b.txt"), "作業/my proj/a b.txt");
    }

    #[test]
    fn rel_path_rejects_escape() {
        for p in ["../x", "a/../../x", "a/..", "..", "C:\\x", "C:x", "c:/x", "a\0b", "a\\..\\..\\x", "a/C:x", "//?/C:/x", "a/C:/Windows"] {
            assert!(RelPath::new(p).is_err(), "{p:?} は拒む");
        }
    }

    #[test]
    fn rel_path_join_and_to_path() {
        let src = RelPath::new("src").unwrap();
        assert_eq!(src.join("a.rs").unwrap().as_str(), "src/a.rs");
        assert_eq!(RelPath::root().join("a").unwrap().as_str(), "a");
        assert!(src.join("..").is_err());
        let base = Path::new("base");
        assert_eq!(src.join("a.rs").unwrap().to_path(base), base.join("src").join("a.rs"));
        assert_eq!(RelPath::root().to_path(base), base.to_path_buf());
    }

    #[test]
    fn rel_path_deserialize_validates() {
        let ok: RelPath = serde_json::from_str(r#""src/a.rs""#).unwrap();
        assert_eq!(ok.as_str(), "src/a.rs");
        assert!(serde_json::from_str::<RelPath>(r#""../x""#).is_err());
        assert_eq!(serde_json::to_string(&ok).unwrap(), r#""src/a.rs""#);
    }
}
