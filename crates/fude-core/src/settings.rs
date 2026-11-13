//! 設定の値（設計書 §5.11）。設定ファイルを読む仕組みは段階 9 で作る。それまでは既定値で動かす。

/// 最後のプロジェクトを閉じたときの動き。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastProjectClosed {
    /// ホーム画面に戻る。
    Home,
    /// アプリを終了する。
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// 動いているプロセスがあるターミナルやプロジェクトを閉じるときに確認するか（§5.12）。
    pub confirm_close_running: bool,
    pub on_last_project_closed: LastProjectClosed,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { confirm_close_running: true, on_last_project_closed: LastProjectClosed::Home }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let s = Settings::default();
        assert!(s.confirm_close_running);
        assert_eq!(s.on_last_project_closed, LastProjectClosed::Home);
    }
}
