//! 返事待ちを OS の通知（Windows のトースト、macOS の通知センター）で知らせる（10e の設計書）。
//! 判断はこのファイルの純粋な関数に置き、OS の呼び出しは OS ごとの mod に置く。

use std::collections::HashSet;

use fude_core::claude::parse_terminal_id;
use fude_core::workspace::{PaneId, ProjectId, TabId, Workspace, display_title};

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

/// ペインの通知の中身。プロジェクト名と、ペインの名前（端末の題名、無ければシェルの種類）を使う。
pub(crate) fn toast_for_pane(ws: &Workspace, pane_raw: u64) -> Option<Toast> {
    let (project_id, pane_id) = ws.pane_by_raw(pane_raw)?;
    let project = ws.projects().iter().find(|p| p.id == project_id)?;
    let pane = project.tabs.iter().flat_map(|t| &t.panes).find(|p| p.id == pane_id)?;
    let name = pane.title.as_deref().and_then(display_title).unwrap_or_else(|| pane.kind.label());
    Some(toast_for(&project.name, &name, pane_raw))
}

/// クリックされたペインの行き先。ペインが既に閉じていれば None（前に出すだけ）。
pub(crate) fn click_target(ws: &Workspace, pane_raw: u64) -> Option<(ProjectId, TabId, PaneId)> {
    let (project_id, pane_id) = ws.pane_by_raw(pane_raw)?;
    let project = ws.projects().iter().find(|p| p.id == project_id)?;
    let tab = project.tabs.iter().find(|t| t.panes.iter().any(|p| p.id == pane_id))?;
    Some((project_id, tab.id, pane_id))
}

/// 通知を出したペインのうち、もう返事待ちでないもの（通知を消すもの）。
pub(crate) fn cleared(notified: &HashSet<u64>, waiting: impl Fn(u64) -> bool) -> Vec<u64> {
    notified.iter().copied().filter(|&p| !waiting(p)).collect()
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

/// 起動時の準備。macOS ではクリックを受ける delegate を通知センターに付ける。ほかでは何もしない。
pub(crate) fn init(clicks: &ClickSender) {
    #[cfg(target_os = "macos")]
    mac::init(clicks);
    #[cfg(not(target_os = "macos"))]
    let _ = clicks;
}

/// 通知を出す。同じ tag の通知は置き換える。出せないときは黙って出さない。
pub(crate) fn show(toast: &Toast, clicks: &ClickSender) {
    #[cfg(windows)]
    win::show(toast, clicks);
    #[cfg(target_os = "macos")]
    {
        let _ = clicks;
        mac::show(toast);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = (toast, clicks);
}

/// tag の通知を通知センターから取り除く。
pub(crate) fn remove(tag: &str) {
    #[cfg(windows)]
    win::remove(tag);
    #[cfg(target_os = "macos")]
    mac::remove(tag);
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = tag;
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU8, Ordering};

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
    use objc2_foundation::{NSArray, NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    use super::{ClickSender, Toast, pane_of_tag};

    /// 通知の許可の状態。
    const UNKNOWN: u8 = 0;
    const ASKING: u8 = 1;
    const GRANTED: u8 = 2;
    const DENIED: u8 = 3;
    static AUTH: AtomicU8 = AtomicU8::new(UNKNOWN);

    struct Ivars {
        clicks: ClickSender,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "FudeNotificationDelegate"]
        #[ivars = Ivars]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            // willPresentNotification は実装しない（前面のときは出さない）。
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                completion_handler: &block2::DynBlock<dyn Fn()>,
            ) {
                let tag = response.notification().request().identifier().to_string();
                if let Some(pane) = pane_of_tag(&tag) {
                    self.ivars().clicks.unbounded_send(pane).ok();
                }
                completion_handler.call(());
            }
        }
    );

    impl Delegate {
        fn new(clicks: ClickSender) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { clicks });
            // SAFETY: NSObject の init を呼ぶだけ。
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        /// 通知センターの delegate は弱い参照なので、ここで持ち続ける。
        static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
    }

    /// .app の中（bundle identifier がある）でだけ通知センターを使う。外で呼ぶと例外で落ちるため。
    fn available() -> bool {
        NSBundle::mainBundle().bundleIdentifier().is_some()
    }

    pub(super) fn init(clicks: &ClickSender) {
        if !available() {
            return;
        }
        let delegate = Delegate::new(clicks.clone());
        UNUserNotificationCenter::currentNotificationCenter().setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));
    }

    fn add(title: &str, body: &str, tag: &str) {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(tag), &content, None);
        UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, None);
    }

    pub(super) fn show(toast: &Toast) {
        if !available() {
            return;
        }
        match AUTH.load(Ordering::SeqCst) {
            GRANTED => add(&toast.title, &toast.body, &toast.tag),
            UNKNOWN => {
                // 最初の 1 回だけ許可を求め、許可されたらこの通知を出す。
                AUTH.store(ASKING, Ordering::SeqCst);
                let (title, body, tag) = (toast.title.clone(), toast.body.clone(), toast.tag.clone());
                let done = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
                    if granted.as_bool() {
                        AUTH.store(GRANTED, Ordering::SeqCst);
                        add(&title, &body, &tag);
                    } else {
                        AUTH.store(DENIED, Ordering::SeqCst);
                    }
                });
                UNUserNotificationCenter::currentNotificationCenter()
                    .requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &done);
            }
            // 許可を求めている間と、拒否されたあとは出さない。
            _ => {}
        }
    }

    pub(super) fn remove(tag: &str) {
        if !available() {
            return;
        }
        let ids = NSArray::from_retained_slice(&[NSString::from_str(tag)]);
        UNUserNotificationCenter::currentNotificationCenter().removeDeliveredNotificationsWithIdentifiers(&ids);
    }
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
    use fude_core::workspace::ProjectRoot;

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

    fn one_project() -> (Workspace, u64) {
        let mut ws = Workspace::default();
        ws.open_project(ProjectRoot::Local(std::path::PathBuf::from("/x/my-app")));
        let pane = ws.active_project().unwrap().active_tab().unwrap().panes[0].id;
        (ws, pane.raw())
    }

    #[test]
    fn click_target_missing_pane() {
        let (ws, raw) = one_project();
        assert!(click_target(&ws, raw).is_some());
        assert_eq!(click_target(&ws, raw + 100), None);
    }

    #[test]
    fn toast_for_pane_uses_names() {
        let (ws, raw) = one_project();
        let t = toast_for_pane(&ws, raw).unwrap();
        assert_eq!(t.title, "fude — my-app");
        assert_eq!(t.tag, format!("p{raw}"));
        assert!(toast_for_pane(&ws, raw + 100).is_none());
    }

    #[test]
    fn cleared_panes() {
        let notified: HashSet<u64> = [1, 2, 3].into_iter().collect();
        let mut got = cleared(&notified, |p| p == 2);
        got.sort();
        assert_eq!(got, vec![1, 3]);
    }

    #[test]
    fn tag_roundtrip() {
        assert_eq!(pane_of_tag(&toast_for("a", "b", 7).tag), Some(7));
        assert_eq!(pane_of_tag("x7"), None);
    }
}
