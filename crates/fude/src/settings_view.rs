//! 設定パネル（9b の設計書 `2026-11-15-settings-panel-hook-design.md` §5.1〜§5.3）。
//! アプリに 1 つだけ作り、エディタ列の設定のタブか、ホーム画面の代わりに出す。
//! 外観、文字、hook の欄はチュートリアルと共有する（settings_sections。9c §5.4）。カーソルと新しい版の欄はここにだけある。
//! ライセンスのページ（10a）も、設定のページと切り替えてここに出す。

use std::path::PathBuf;

use fude_core::settings::{CursorShape, Settings};
use fude_core::settings_panel::PanelEdit;
use gpui_kit::base::{TextView, TextViewState, TextViewStyle};
use gpui_kit::*;

use crate::config_runtime::{ConfigState, apply_edit};
use crate::hook_targets::HookBoard;
use crate::settings_sections::{AppearanceSection, HookSection, SectionMode, TextSection, chip, heading, row, small_button, toggle};
use crate::theme::{Theme, ui_font};

/// 設定パネルが WorkspaceView に頼むこと。
pub enum SettingsViewEvent {
    /// 設定ファイルを開く。
    OpenConfig(PathBuf),
}

impl EventEmitter<SettingsViewEvent> for SettingsView {}

/// 出しているページ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Main,
    Licenses,
}

pub struct SettingsView {
    focus: FocusHandle,
    scroll: ScrollHandle,
    page: Page,
    /// ライセンスの文書。大きい（数百 KB）ので、初めて開いたときに作って使い回す。
    licenses: Option<Entity<TextViewState>>,
    appearance: Entity<AppearanceSection>,
    text: Entity<TextSection>,
    hooks: Entity<HookSection>,
    /// Claude Code の hook の登録先と、その状態（チュートリアルと共有する。9c §5.4）。
    board: Entity<HookBoard>,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(board: Entity<HookBoard>, window: &mut Window, cx: &mut Context<Self>) -> SettingsView {
        let appearance = cx.new(|cx| AppearanceSection::new(SectionMode::Full, window, cx));
        let text = cx.new(|cx| TextSection::new(SectionMode::Full, window, cx));
        let hooks = cx.new(|cx| HookSection::new(board.clone(), SectionMode::Full, cx));
        let subs = vec![cx.observe_global::<ConfigState>(|_, cx| cx.notify())];
        SettingsView { focus: cx.focus_handle(), scroll: ScrollHandle::new(), page: Page::Main, licenses: None, appearance, text, hooks, board, _subs: subs }
    }

    /// パネルを開き直した。hook の状態を調べ直す。
    pub fn reopened(&mut self, cx: &mut Context<Self>) {
        self.board.update(cx, |b, cx| b.recheck(cx));
        self.show_main(cx);
    }

    /// ライセンスのページを出す。
    pub fn show_licenses(&mut self, cx: &mut Context<Self>) {
        if self.licenses.is_none() {
            let doc = fude_core::licenses::document();
            self.licenses = Some(cx.new(|cx| TextViewState::markdown(&doc, cx)));
        }
        self.page = Page::Licenses;
        cx.notify();
    }

    /// 設定のページに戻す。
    pub fn show_main(&mut self, cx: &mut Context<Self>) {
        if self.page != Page::Main {
            self.page = Page::Main;
            self.scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        }
    }

    fn render_licenses(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let back = small_button("back-to-settings", "← 設定に戻る", theme).on_click(cx.listener(|this, _, _, cx| this.show_main(cx)));
        let mut page = div().size_full().flex().flex_col().px(px(24.)).pt(px(16.)).child(div().flex().pb(px(12.)).child(back));
        if let Some(state) = &self.licenses {
            let view = TextView::new(state)
                .scrollable(true)
                .selectable(true)
                .style(TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx)).with_link(theme.accent.into()))
                .on_link_click(|url, _, _, cx| cx.open_url(url));
            page = page.child(div().flex_1().min_h_0().font(ui_font()).text_size(px(13.)).child(view));
        }
        page
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn render_cursor(&self, s: &Settings, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let shapes = [(CursorShape::Block, "ブロック"), (CursorShape::Bar, "縦線"), (CursorShape::Underline, "下線")];
        let shape = div().flex().flex_row().gap(px(6.)).children(shapes.into_iter().map(|(shape, label)| {
            chip(SharedString::from(format!("cursor-{label}")), label, s.cursor.shape == shape, theme)
                .on_click(cx.listener(move |_, _, _, cx| apply_edit(cx, PanelEdit::CursorShape(shape))))
        }));
        let blink_on = s.cursor.blink;
        let blink = toggle("blink", blink_on, true, theme)
            .on_click(cx.listener(move |_, _, _, cx| apply_edit(cx, PanelEdit::CursorBlink(!blink_on))));
        div().child(heading("カーソル", theme)).child(row("形", shape, theme)).child(row("点滅", blink, theme))
    }

    /// 新しい版の欄（10g）。設定パネルにだけある。
    fn render_updates(&self, s: &Settings, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let on = s.updates.check;
        let check = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .child(toggle("update-check", on, true, theme).on_click(cx.listener(move |_, _, _, cx| apply_edit(cx, PanelEdit::UpdateCheck(!on)))))
            .child(div().text_color(theme.fg_dim).child("毎日 GitHub の Release をチェックします。"));
        div().child(heading("新しい版", theme)).child(row("新しい版を確かめる", check, theme))
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        if self.page == Page::Licenses {
            return div()
                .id("settings")
                .track_focus(&self.focus)
                .key_context("Settings")
                .size_full()
                .text_color(theme.fg_default)
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                    if ev.keystroke.key == "escape" {
                        this.show_main(cx);
                        cx.stop_propagation();
                    }
                }))
                .child(self.render_licenses(&theme, cx));
        }
        let state = cx.global::<ConfigState>().clone();
        let cursor = self.render_cursor(&state.settings, &theme, cx);
        let updates = self.render_updates(&state.settings, &theme, cx);
        let licenses = div().child(heading("ライセンス", &theme)).child(row(
            "fude と同梱物",
            small_button("open-licenses", "ライセンスを表示", &theme).on_click(cx.listener(|this, _, _, cx| this.show_licenses(cx))),
            &theme,
        ));
        let open_config = state.path.clone().map(|path| {
            div().pt(px(24.)).flex().child(
                small_button("open-config", "設定ファイルを開く", &theme)
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(SettingsViewEvent::OpenConfig(path.clone())))),
            )
        });
        div()
            .id("settings")
            .track_focus(&self.focus)
            .key_context("Settings")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(24.))
            .pb(px(32.))
            .text_size(px(13.))
            .text_color(theme.fg_default)
            .children(state.write_error.clone().map(|e| div().mt(px(12.)).text_color(theme.warn).child(e)))
            .child(self.appearance.clone())
            .child(self.text.clone())
            .child(cursor)
            .child(self.hooks.clone())
            .child(updates)
            .child(licenses)
            .children(open_config)
    }
}
