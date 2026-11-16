//! 新しい版を確かめるための判断（版の比べ方、24 時間の判定、JSON の読み取り、覚えておくファイル、印を出すか）。
//! 通信そのものは `fude` 側で行い、ここには純粋な関数だけを置く。

use std::cmp::Ordering;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::keymap::Platform;

/// 問い合わせ先（GitHub Releases の最新版。下書きとプレリリースは返らない）。
pub const LATEST_URL: &str = "https://api.github.com/repos/Nek0cha/fude/releases/latest";

/// 確かめる間隔（24 時間）。
const INTERVAL_SECS: u64 = 24 * 60 * 60;

/// 読み取った版。数字の 3 つ組で比べ、同じなら `pre: true`（`-rc.1` など）を古いとみなす。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: bool,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            // 正式版（pre: false）の方を大きくする。
            .then_with(|| other.pre.cmp(&self.pre))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 版の文字を読む。先頭の `v` と `+` 以降（ビルドの情報）を除き、`X.Y.Z` か `X.Y.Z-…` だけを受け付ける。
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    let s = s.split('+').next().unwrap_or(s);
    let (core, pre) = match s.split_once('-') {
        Some((core, rest)) => (core, !rest.is_empty()),
        None => (s, false),
    };
    let mut parts = core.split('.');
    let mut num = || -> Option<u64> {
        let p = parts.next()?;
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse().ok()
    };
    let (major, minor, patch) = (num()?, num()?, num()?);
    if parts.next().is_some() {
        return None;
    }
    Some(Version { major, minor, patch, pre })
}

/// `latest` が `current` より新しいか。どちらかが読めなければ false。
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// 見つけた最新版（`tag_name` と `html_url`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
}

/// GitHub の API の答えを読む。下書きかプレリリースなら None。
pub fn parse_latest(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let flag = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    if flag("draft") || flag("prerelease") {
        return None;
    }
    let version = v.get("tag_name")?.as_str()?.to_string();
    let url = v.get("html_url")?.as_str()?.to_string();
    Some(Release { version, url })
}

/// `update-check.json` に覚えておくもの。時刻は UNIX 秒。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckState {
    #[serde(default)]
    pub last_checked: Option<u64>,
    #[serde(default)]
    pub latest: Option<Release>,
    #[serde(default)]
    pub dismissed: Option<String>,
}

/// ファイルの中身を読む。壊れていれば無いものとして扱う。
pub fn load_state(text: &str) -> CheckState {
    serde_json::from_str(text).unwrap_or_default()
}

/// ファイルに書く中身を作る。
pub fn save_state(s: &CheckState) -> String {
    serde_json::to_string_pretty(s).unwrap_or_default()
}

/// 問い合わせる時か。まだ確かめていない、24 時間以上たった、最後の時刻が未来（時計が戻った）なら true。
pub fn due(last_checked: Option<u64>, now: u64) -> bool {
    match last_checked {
        None => true,
        Some(t) if t > now => true,
        Some(t) => now - t >= INTERVAL_SECS,
    }
}

/// 印に出す版。最新版が今の版より新しく、× で消した版がその版でも、それより新しい版でもないとき。
pub fn badge(state: &CheckState, current: &str) -> Option<Release> {
    let latest = state.latest.as_ref()?;
    if !is_newer(&latest.version, current) {
        return None;
    }
    if let Some(d) = &state.dismissed {
        if d == &latest.version {
            return None;
        }
        // 読めない版を消してあるときは、最新版を出す（最新版は読めることを上で確かめた）。
        if let (Some(dv), Some(lv)) = (parse_version(d), parse_version(&latest.version))
            && dv >= lv
        {
            return None;
        }
    }
    Some(latest.clone())
}

/// `update-check.json` の場所（`recent` や `session` と同じフォルダ）。
pub fn state_path(os: Platform, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    crate::session::data_file(os, env, "update-check", "json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_cases() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.2.0", "0.1.0+1794818938"));
        assert!(!is_newer("v0.1.0", "0.1.0+1794818938"));
        assert!(is_newer("v0.2.0", "0.2.0-rc.1"));
        assert!(!is_newer("v0.2.0-rc.2", "0.2.0"));
        assert!(is_newer("v1.10.0", "1.9.0"));
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("v0.2", "0.1.0"));
    }

    #[test]
    fn parse_latest_cases() {
        let j = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/Nek0cha/fude/releases/tag/v0.2.0","draft":false,"prerelease":false}"#;
        assert_eq!(parse_latest(j), Some(Release { version: "v0.2.0".into(), url: "https://github.com/Nek0cha/fude/releases/tag/v0.2.0".into() }));
        assert_eq!(parse_latest(r#"{"tag_name":"v0.2.0","html_url":"u","prerelease":true}"#), None);
        assert_eq!(parse_latest("not json"), None);
    }

    #[test]
    fn due_cases() {
        assert!(due(None, 1000));
        assert!(!due(Some(1000), 1000 + 86_399));
        assert!(due(Some(1000), 1000 + 86_400));
        assert!(due(Some(5000), 1000)); // 未来の時刻
    }

    #[test]
    fn state_roundtrip_and_broken() {
        let s = CheckState { last_checked: Some(1), latest: Some(Release { version: "v0.2.0".into(), url: "u".into() }), dismissed: None };
        assert_eq!(load_state(&save_state(&s)), s);
        assert_eq!(load_state("{broken"), CheckState::default());
    }

    #[test]
    fn badge_after_dismiss() {
        let mut s = CheckState { last_checked: Some(1), latest: Some(Release { version: "v0.2.0".into(), url: "u".into() }), dismissed: None };
        assert!(badge(&s, "0.1.0").is_some());
        s.dismissed = Some("v0.2.0".into());
        assert!(badge(&s, "0.1.0").is_none());
        s.latest = Some(Release { version: "v0.3.0".into(), url: "u3".into() });
        assert_eq!(badge(&s, "0.1.0").map(|r| r.url), Some("u3".into()));
        assert!(badge(&s, "0.3.0").is_none());
    }
}
