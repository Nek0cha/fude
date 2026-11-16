//! 新しい版の問い合わせ（10g）。GitHub Releases の最新版を 1 回だけ取りに行く。
//! 判断（版の比べ方、24 時間の判定など）は fude_core::update_check にある。ここは通信だけで、試験しない。

use std::path::PathBuf;
use std::time::Duration;

use fude_core::keymap::Platform;
use fude_core::update_check::{CheckState, LATEST_URL, Release, badge, due, load_state, parse_latest, save_state, state_path};
use gpui_kit::assets::IconName;
use gpui_kit::*;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::theme::Theme;
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

/// 今の fude の版。
pub(crate) const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// 時間切れ（接続から読み終わりまで）。
const TIMEOUT: Duration = Duration::from_secs(10);
/// 読む本文の上限。
const BODY_LIMIT: u64 = 1024 * 1024;
/// ウィンドウを出してから問い合わせるまで待つ時間（起動を遅くしない）。
const CHECK_DELAY: Duration = Duration::from_secs(5);

/// 最新版を問い合わせる。通信の失敗、時間切れ、読めない答えはすべて None（黙って捨てる）。
/// 裏のスレッドで呼ぶ（待つ間スレッドを止める）。
pub(crate) fn fetch_latest() -> Option<Release> {
    // OS の TLS（Windows は SChannel、mac は Security.framework）と、OS の証明書を使う。
    let tls = TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build();
    let agent: ureq::Agent = ureq::Agent::config_builder().tls_config(tls).timeout_global(Some(TIMEOUT)).build().into();
    let mut response = agent
        .get(LATEST_URL)
        .header("User-Agent", format!("fude/{CURRENT}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?;
    let text = response.body_mut().with_config().limit(BODY_LIMIT).read_to_string().ok()?;
    parse_latest(&text)
}

/// 新しい版の知らせの状態（WorkspaceView が 1 つ持つ）。
#[derive(Default)]
pub(crate) struct UpdateNotice {
    /// タイトル行に出す版。None なら出さない。
    pub(crate) badge: Option<Release>,
    /// update-check.json の中身。
    state: CheckState,
    /// update-check.json の場所。環境変数から決まらなければ読み書きしない。
    path: Option<PathBuf>,
    /// 待ってから問い合わせる処理。捨てると取り消される。
    task: Option<Task<()>>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl WorkspaceView {
    /// 起動時に呼ぶ。前回の結果をすぐ印にし、24 時間たっていれば少し待ってから裏で問い合わせる。
    pub(crate) fn start_update_check(&mut self, cx: &mut Context<Self>) {
        if !self.settings.updates.check {
            return;
        }
        let env = |k: &str| std::env::var(k).ok();
        let Some(path) = state_path(Platform::current(), &env) else { return };
        // 読めない・壊れているときは無いものとして扱う。
        let state = std::fs::read_to_string(&path).map(|t| load_state(&t)).unwrap_or_default();
        self.update.badge = badge(&state, CURRENT);
        let due = due(state.last_checked, now_secs());
        self.update.state = state;
        self.update.path = Some(path);
        if !due {
            return;
        }
        self.update.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHECK_DELAY).await;
            let Some(release) = cx.background_spawn(async { fetch_latest() }).await else { return };
            this.update(cx, |this, cx| this.on_latest_release(release, cx)).ok();
        }));
    }

    /// 問い合わせの答えが来た。覚えて、印を出し直す。
    fn on_latest_release(&mut self, release: Release, cx: &mut Context<Self>) {
        self.update.task = None;
        if !self.settings.updates.check {
            return;
        }
        self.update.state.latest = Some(release);
        self.update.state.last_checked = Some(now_secs());
        self.save_update_state(cx);
        self.update.badge = badge(&self.update.state, CURRENT);
        cx.notify();
    }

    /// 印の × を押した。その版の間は出さない。
    pub(crate) fn dismiss_update(&mut self, cx: &mut Context<Self>) {
        let Some(release) = self.update.badge.take() else { return };
        self.update.state.dismissed = Some(release.version);
        self.save_update_state(cx);
        cx.notify();
    }

    /// 設定の [updates] が変わった。切ったら印を消して問い合わせを取り消し、入れたら確かめ始める。
    pub(crate) fn on_update_setting_changed(&mut self, cx: &mut Context<Self>) {
        self.update.badge = None;
        self.update.task = None;
        if self.settings.updates.check {
            self.start_update_check(cx);
        }
        cx.notify();
    }

    /// update-check.json に裏で書く。失敗は黙って捨てる。
    fn save_update_state(&self, cx: &mut Context<Self>) {
        let Some(path) = self.update.path.clone() else { return };
        let text = save_state(&self.update.state);
        cx.background_spawn(async move {
            let _ = fude_core::session::write_atomic(&path, &text);
        })
        .detach();
    }

    /// タイトル行の「新しい版 X.Y.Z」と ×。押すと Releases のページを開く。
    pub(crate) fn render_update_badge(&self, cx: &mut Context<Self>) -> Option<Div> {
        let release = self.update.badge.clone()?;
        let theme = *cx.global::<Theme>();
        let label = format!("新しい版 {}", release.version.trim_start_matches('v'));
        let url = release.url;
        Some(
            div()
                .h(px(24.))
                .pl(px(10.))
                .pr(px(3.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.))
                .flex_shrink_0()
                .rounded(px(12.))
                .border_1()
                .border_color(theme.accent)
                .text_size(px(12.))
                .child(
                    div()
                        .id("update-open")
                        .cursor_pointer()
                        .text_color(theme.accent)
                        .whitespace_nowrap()
                        .child(label)
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .child(
                    div()
                        .id("update-dismiss")
                        .size(px(18.))
                        .rounded(px(9.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.bg_hover))
                        .child(icon(IconName::X, theme.fg_dim).size(px(10.)))
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss_update(cx))),
                ),
        )
    }
}
