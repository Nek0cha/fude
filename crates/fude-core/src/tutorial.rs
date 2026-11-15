//! 初回のチュートリアルを出すかの判定と、「見た」の印（9c の設計書 `2026-11-15-tutorial-design.md` §5.1、§5.5）。

use std::path::PathBuf;

use crate::keymap::Platform;
use crate::session::data_file;

/// 印のファイルの中身。
pub const DONE_TEXT: &str = "{\"done\": true}\n";

/// 印のファイルの場所（`FUDE_INSTANCE` があれば名前に付く）。
pub fn tutorial_path(os: Platform, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    data_file(os, env, "tutorial", "json")
}

/// 印のファイルの中身が「見た」か。無い、読めない、壊れているときは false。
pub fn is_done(text: Option<&str>) -> bool {
    text.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
        .is_some_and(|v| v.get("done") == Some(&serde_json::Value::Bool(true)))
}

/// 起動時にどうするか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupTutorial {
    /// ホーム画面の代わりに出す。
    Show,
    /// 出さない（印は触らない）。
    Hide,
    /// 9c より前から使っている人。出さずに印を付ける。
    MarkDone,
}

/// home_launch は引数なしの起動（ホーム画面が出る起動）か。done は印があるか。recent_empty は最近の一覧が空か。
pub fn startup_tutorial(home_launch: bool, done: bool, recent_empty: bool) -> StartupTutorial {
    match (home_launch, done, recent_empty) {
        (false, _, _) | (_, true, _) => StartupTutorial::Hide,
        (true, false, true) => StartupTutorial::Show,
        (true, false, false) => StartupTutorial::MarkDone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Platform;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn done_mark_round_trip() {
        assert!(is_done(Some(DONE_TEXT)));
        for text in [None, Some(""), Some("{"), Some("{\"done\": false}"), Some("[]")] {
            assert!(!is_done(text), "{text:?}");
        }
    }

    #[test]
    fn startup_cases() {
        assert_eq!(startup_tutorial(false, false, true), StartupTutorial::Hide);
        assert_eq!(startup_tutorial(true, true, true), StartupTutorial::Hide);
        assert_eq!(startup_tutorial(true, false, true), StartupTutorial::Show);
    }

    #[test]
    fn existing_user_is_marked_silently() {
        assert_eq!(startup_tutorial(true, false, false), StartupTutorial::MarkDone);
    }

    /// 途中で閉じた人は、config.toml ができていても最近の一覧が空のままなので、また出る。
    #[test]
    fn closed_midway_shows_again() {
        assert_eq!(startup_tutorial(true, false, true), StartupTutorial::Show);
    }

    #[test]
    fn tutorial_path_uses_instance() {
        let mut env: HashMap<&str, &str> = HashMap::from([("APPDATA", r"C:\a")]);
        let get = |env: &HashMap<&str, &str>| {
            let env = env.clone();
            tutorial_path(Platform::Windows, &move |k| env.get(k).map(|v| v.to_string()))
        };
        assert_eq!(get(&env), Some(PathBuf::from(r"C:\a").join("fude").join("tutorial.json")));
        env.insert("FUDE_INSTANCE", "dev");
        assert_eq!(get(&env), Some(PathBuf::from(r"C:\a").join("fude").join("tutorial-dev.json")));
    }
}
