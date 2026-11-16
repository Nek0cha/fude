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

/// 通知のクリックを main のスレッドへ送る口。送るのはペインの番号。
pub(crate) type ClickSender = futures::channel::mpsc::UnboundedSender<u64>;

/// XML の文字の実体参照。
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Windows のトーストの XML。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn toast_xml(t: &Toast) -> String {
    format!(
        "<toast launch=\"{}\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(&t.tag),
        xml_escape(&t.title),
        xml_escape(&t.body)
    )
}

/// 通知を出す。同じ tag の通知は置き換える。出せないときは黙って出さない。
pub(crate) fn show(toast: &Toast, clicks: &ClickSender) {
    #[cfg(windows)]
    win::show(toast, clicks);
    #[cfg(not(windows))]
    let _ = (toast, clicks);
}

/// tag の通知を通知センターから取り除く。
pub(crate) fn remove(tag: &str) {
    #[cfg(windows)]
    win::remove(tag);
    #[cfg(not(windows))]
    let _ = tag;
}

#[cfg(windows)]
mod win {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::{HSTRING, IInspectable};

    use super::{ClickSender, Toast, pane_of_tag, toast_xml};
    use crate::APP_USER_MODEL_ID;

    /// 通知のまとまりの名前。
    const GROUP: &str = "waiting";

    thread_local! {
        /// 出した通知を持っておく（手放すとクリックのイベントが届かなくなるため）。UI のスレッドだけで使う。
        static SHOWN: RefCell<HashMap<String, ToastNotification>> = RefCell::new(HashMap::new());
    }

    fn try_show(toast: &Toast, clicks: &ClickSender) -> windows::core::Result<ToastNotification> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(toast_xml(toast)))?;
        let n = ToastNotification::CreateToastNotification(&doc)?;
        n.SetTag(&HSTRING::from(toast.tag.as_str()))?;
        n.SetGroup(&HSTRING::from(GROUP))?;
        let clicks = clicks.clone();
        let tag = toast.tag.clone();
        n.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
            if let Some(pane) = pane_of_tag(&tag) {
                clicks.unbounded_send(pane).ok();
            }
            Ok(())
        }))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_USER_MODEL_ID))?.Show(&n)?;
        Ok(n)
    }

    pub(super) fn show(toast: &Toast, clicks: &ClickSender) {
        match try_show(toast, clicks) {
            Ok(n) => {
                SHOWN.with(|m| m.borrow_mut().insert(toast.tag.clone(), n));
            }
            Err(e) => eprintln!("通知を出せない: {e}"),
        }
    }

    pub(super) fn remove(tag: &str) {
        let had = SHOWN.with(|m| m.borrow_mut().remove(tag)).is_some();
        if !had {
            return;
        }
        let result = ToastNotificationManager::History().and_then(|h| {
            h.RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(GROUP), &HSTRING::from(APP_USER_MODEL_ID))
        });
        if let Err(e) = result {
            eprintln!("通知を消せない: {e}");
        }
    }
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
    fn toast_xml_escapes() {
        let t = Toast { title: "fude — a&b".into(), body: "<x> \"y\" 'z'".into(), tag: "p42".into() };
        assert_eq!(
            toast_xml(&t),
            "<toast launch=\"p42\"><visual><binding template=\"ToastGeneric\"><text>fude — a&amp;b</text><text>&lt;x&gt; &quot;y&quot; &apos;z&apos;</text></binding></visual></toast>"
        );
    }

    #[test]
    fn tag_roundtrip() {
        assert_eq!(pane_of_tag(&toast_for("a", "b", 7).tag), Some(7));
        assert_eq!(pane_of_tag("x7"), None);
    }
}
