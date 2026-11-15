//! 初回のチュートリアル（9c の設計書 `2026-11-15-tutorial-design.md`）。

use std::path::Path;

use fude_core::tutorial::{DONE_TEXT, StartupTutorial, is_done, startup_tutorial};

/// 「見た」の印を書く（親のフォルダが無ければ作る）。
pub(crate) fn write_mark(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, DONE_TEXT)
}

/// 起動時にチュートリアルを出すかを決める（§5.1）。9c より前から使っている人には、その場で印を付ける（失敗は無視）。
pub(crate) fn decide_startup(home_launch: bool, mark: Option<&Path>, recent_empty: bool) -> StartupTutorial {
    let done = is_done(mark.and_then(|p| std::fs::read_to_string(p).ok()).as_deref());
    let decision = startup_tutorial(home_launch, done, recent_empty);
    if decision == StartupTutorial::MarkDone
        && let Some(p) = mark
    {
        write_mark(p).ok();
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::{decide_startup, write_mark};
    use fude_core::tutorial::{StartupTutorial, is_done};

    #[test]
    fn startup_decision_for_existing_user() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("fude").join("tutorial.json");
        assert_eq!(decide_startup(true, Some(&mark), false), StartupTutorial::MarkDone);
        assert!(is_done(std::fs::read_to_string(&mark).ok().as_deref()));
        // 印が付いたので、次は出さない。
        assert_eq!(decide_startup(true, Some(&mark), true), StartupTutorial::Hide);
    }

    #[test]
    fn startup_decision_for_new_user() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("tutorial.json");
        assert_eq!(decide_startup(true, Some(&mark), true), StartupTutorial::Show);
        assert!(!mark.exists(), "出すときは印を付けない");
        assert_eq!(decide_startup(false, Some(&mark), true), StartupTutorial::Hide);
    }

    #[test]
    fn write_mark_reports_error_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file.txt");
        std::fs::write(&blocker, "").unwrap();
        assert!(write_mark(&blocker.join("tutorial.json")).is_err());
    }
}
