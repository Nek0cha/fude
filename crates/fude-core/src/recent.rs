//! 最近開いたプロジェクトの記録（設計書 §5.10）。ホーム画面の「最近」に出す。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::keymap::Platform;
use crate::session::data_file;
use crate::workspace::ProjectRoot;

/// 保存する件数の上限。
pub const RECENT_MAX: usize = 50;
/// ホーム画面に出す件数。
pub const HOME_RECENT: usize = 10;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RecentEntry {
    pub root: ProjectRoot,
    /// 最後に開いた日時（UNIX 秒）。
    pub opened_at: u64,
}

/// 新しく開いた順に並べた一覧。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RecentList {
    entries: Vec<RecentEntry>,
}

impl RecentList {
    /// root を先頭に置く。同じ場所（same_place）があれば取り除いてから置く。RECENT_MAX を超えた分は捨てる。
    pub fn touch(&mut self, root: ProjectRoot, now: u64) {
        self.entries.retain(|e| !e.root.same_place(&root));
        self.entries.insert(0, RecentEntry { root, opened_at: now });
        self.entries.truncate(RECENT_MAX);
    }

    pub fn remove(&mut self, root: &ProjectRoot) {
        self.entries.retain(|e| !e.root.same_place(root));
    }

    /// 新しいものから n 件。
    pub fn top(&self, n: usize) -> &[RecentEntry] {
        &self.entries[..n.min(self.entries.len())]
    }

    /// 壊れていれば空。
    pub fn parse(text: &str) -> RecentList {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("RecentList は必ず JSON にできる")
    }
}

/// セッションと同じディレクトリの recent.json（FUDE_INSTANCE=x なら recent-x.json）。
pub fn recent_path(os: Platform, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    data_file(os, env, "recent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Platform;
    use crate::workspace::ProjectRoot;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn local(p: &str) -> ProjectRoot {
        ProjectRoot::Local(PathBuf::from(p))
    }

    fn wsl(distro: &str, path: &str) -> ProjectRoot {
        ProjectRoot::Wsl { distro: distro.into(), path: path.into() }
    }

    #[test]
    fn touch_moves_to_front_and_dedups() {
        let mut r = RecentList::default();
        r.touch(local("/w/a"), 1);
        r.touch(wsl("Ubuntu", "/home/x"), 2);
        r.touch(wsl("ubuntu", "/home/x"), 3);
        r.touch(local("/w/a"), 4);
        let roots: Vec<_> = r.top(10).iter().map(|e| (e.root.clone(), e.opened_at)).collect();
        assert_eq!(roots, vec![(local("/w/a"), 4), (wsl("ubuntu", "/home/x"), 3)]);
    }

    #[test]
    fn keeps_at_most_50() {
        let mut r = RecentList::default();
        for i in 0..60 {
            r.touch(local(&format!("/w/{i}")), i);
        }
        assert_eq!(r.top(usize::MAX).len(), RECENT_MAX);
        assert_eq!(r.top(1)[0].root, local("/w/59"));
        assert!(r.top(usize::MAX).iter().all(|e| e.opened_at >= 10));
    }

    #[test]
    fn remove_and_top() {
        let mut r = RecentList::default();
        for i in 0..12 {
            r.touch(local(&format!("/w/{i}")), i);
        }
        r.remove(&local("/w/11"));
        let top = r.top(HOME_RECENT);
        assert_eq!(top.len(), 10);
        assert_eq!(top[0].root, local("/w/10"));
        assert_eq!(top[9].root, local("/w/1"));
    }

    #[test]
    fn parse_broken_is_empty() {
        assert!(RecentList::parse("{").top(10).is_empty());
        let mut r = RecentList::default();
        r.touch(wsl("Ubuntu", "/home/x"), 5);
        assert_eq!(RecentList::parse(&r.to_json()), r);
    }

    #[test]
    fn recent_path_follows_instance() {
        let vars: HashMap<&str, &str> = [("APPDATA", r"C:\U\AppData\Roaming"), ("HOME", "/Users/u")].into();
        let env = |k: &str| vars.get(k).map(|v| v.to_string());
        assert_eq!(
            recent_path(Platform::Windows, &env),
            Some(PathBuf::from(r"C:\U\AppData\Roaming").join("fude").join("recent.json"))
        );
        assert_eq!(
            recent_path(Platform::MacOs, &env),
            Some(PathBuf::from("/Users/u").join(".config").join("fude").join("recent.json"))
        );
        let dev: HashMap<&str, &str> = [("APPDATA", r"C:\A"), ("FUDE_INSTANCE", "dev")].into();
        let env = |k: &str| dev.get(k).map(|v| v.to_string());
        assert_eq!(recent_path(Platform::Windows, &env), Some(PathBuf::from(r"C:\A").join("fude").join("recent-dev.json")));
    }
}
