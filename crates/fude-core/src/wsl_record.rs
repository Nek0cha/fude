//! WSL に置いた fude の記録（10b の設計書 §5.4）。アンインストールで、記録にあるものだけを片付ける。

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::keymap::Platform;

/// WSL に置いた fude 1 つ（ディストロと、その中の置き場所）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployed {
    pub distro: String,
    pub bin: String,
}

/// 記録のファイル。Windows は %APPDATA%\fude\wsl-deployed.json（FUDE_INSTANCE=x なら wsl-deployed-x.json）。
pub fn record_path(os: Platform, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    crate::session::data_file(os, env, "wsl-deployed", "json")
}

/// 無い、読めない、壊れている → 空。
pub fn load(path: &Path) -> Vec<Deployed> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// 同じ組（ディストロ名は大文字小文字を区別しない）が無ければ足す。足したら true。
pub fn add(list: &mut Vec<Deployed>, distro: &str, bin: &str) -> bool {
    if list.iter().any(|d| d.distro.eq_ignore_ascii_case(distro) && d.bin == bin) {
        return false;
    }
    list.push(Deployed { distro: distro.to_string(), bin: bin.to_string() });
    true
}

/// 記録に足す。足したときだけ書く。
pub fn remember(path: &Path, distro: &str, bin: &str) -> io::Result<()> {
    let mut list = load(path);
    if !add(&mut list, distro, bin) {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&list).map_err(io::Error::other)?;
    crate::session::write_atomic(path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn add_skips_same_pair() {
        let mut list = Vec::new();
        assert!(add(&mut list, "Ubuntu", "~/.local/bin/fude"));
        assert!(!add(&mut list, "ubuntu", "~/.local/bin/fude"));
        assert!(add(&mut list, "Ubuntu", "/tmp/x/fude"));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn load_treats_missing_or_broken_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&dir.path().join("none.json")).is_empty());
        std::fs::write(dir.path().join("broken.json"), "{not json").unwrap();
        assert!(load(&dir.path().join("broken.json")).is_empty());
        std::fs::write(dir.path().join("empty.json"), "").unwrap();
        assert!(load(&dir.path().join("empty.json")).is_empty());
    }

    #[test]
    fn remember_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("wsl-deployed.json");
        remember(&path, "Ubuntu", "~/.local/bin/fude").unwrap();
        remember(&path, "Ubuntu", "~/.local/bin/fude").unwrap();
        assert_eq!(load(&path), vec![Deployed { distro: "Ubuntu".into(), bin: "~/.local/bin/fude".into() }]);
    }

    #[test]
    fn record_path_follows_instance() {
        let base: HashMap<&str, &str> = [("APPDATA", r"C:\A")].into();
        let env = |k: &str| base.get(k).map(|v| v.to_string());
        assert_eq!(record_path(Platform::Windows, &env), Some(PathBuf::from(r"C:\A").join("fude").join("wsl-deployed.json")));
        let dev: HashMap<&str, &str> = [("APPDATA", r"C:\A"), ("FUDE_INSTANCE", "dev")].into();
        let env = |k: &str| dev.get(k).map(|v| v.to_string());
        assert_eq!(record_path(Platform::Windows, &env), Some(PathBuf::from(r"C:\A").join("fude").join("wsl-deployed-dev.json")));
    }
}
