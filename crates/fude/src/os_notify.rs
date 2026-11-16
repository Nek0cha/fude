//! 返事待ちを OS の通知（Windows のトースト、macOS の通知センター）で知らせる（10e の設計書）。
//! 判断はこのファイルの純粋な関数に置き、OS の呼び出しは OS ごとの mod に置く。

use fude_core::claude::parse_terminal_id;

/// 新しく返事待ちになり、ウィンドウが前面でなく、設定で切っていないときだけ出す。
pub(crate) fn should_notify(became_waiting: bool, window_active: bool, enabled: bool) -> bool {
    became_waiting && !window_active && enabled
}

/// 出す通知の中身。tag はペインの番号（`p<数字>`）で、同じペインの通知を置き換え・消すのに使う。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Toast {
    pub title: String,
    pub body: String,
    pub tag: String,
}

pub(crate) fn toast_for(project: &str, tab: &str, pane_raw: u64) -> Toast {
    Toast {
        title: format!("fude — {project}"),
        body: format!("Claude Code が返事を待っています（{tab}）"),
        tag: format!("p{pane_raw}"),
    }
}

/// tag からペインの番号に戻す。
pub(crate) fn pane_of_tag(tag: &str) -> Option<u64> {
    parse_terminal_id(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_notify_cases() {
        assert!(should_notify(true, false, true));
        // 前面。
        assert!(!should_notify(true, true, true));
        // 新しい返事待ちではない。
        assert!(!should_notify(false, false, true));
        // 設定で切った。
        assert!(!should_notify(true, false, false));
    }

    #[test]
    fn toast_text() {
        let t = toast_for("my-app", "ターミナル 2", 42);
        assert_eq!(t.title, "fude — my-app");
        assert_eq!(t.body, "Claude Code が返事を待っています（ターミナル 2）");
        assert_eq!(t.tag, "p42");
    }

    #[test]
    fn tag_roundtrip() {
        assert_eq!(pane_of_tag(&toast_for("a", "b", 7).tag), Some(7));
        assert_eq!(pane_of_tag("x7"), None);
    }
}
