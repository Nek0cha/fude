//! 設定パネル（9b の設計書 `2026-11-15-settings-panel-hook-design.md` §5.1〜§5.3）。
//! アプリに 1 つだけ作り、エディタ列の設定のタブか、ホーム画面の代わりに出す。

use gpui_kit::*;

use crate::theme::Theme;

pub struct SettingsView {
    focus: FocusHandle,
}

impl SettingsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> SettingsView {
        SettingsView { focus: cx.focus_handle() }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        div().id("settings").track_focus(&self.focus).key_context("Settings").size_full().p_4().text_color(theme.fg_default).child("設定")
    }
}
