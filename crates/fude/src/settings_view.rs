//! 設定パネル（9b の設計書 `2026-11-15-settings-panel-hook-design.md` §5.1〜§5.3）。
//! アプリに 1 つだけ作り、エディタ列の設定のタブか、ホーム画面の代わりに出す。
//! 値は ConfigState を見て描き、変えたら config_runtime::apply_edit に渡す（画面への反映とファイルへの書き込み）。

use std::path::PathBuf;

use fude_core::settings::{CursorShape, FontSettings, Settings, ThemeId};
use fude_core::settings_panel::{FONT_SIZE_MAX, FONT_SIZE_MIN, LINE_HEIGHT_MAX, OPACITY_MAX, OPACITY_MIN, PanelEdit};
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::Input;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::config_runtime::{ConfigState, apply_edit};
use crate::theme::{Theme, size};

/// 書体の候補を出す数。
const FONT_MATCHES: usize = 8;
/// ぼかしを使える OS。
const BLUR_SUPPORTED: bool = cfg!(any(windows, target_os = "macos"));

/// 0.62 → "62%"。
pub(crate) fn percent_label(opacity: f32) -> String {
    format!("{}%", (opacity * 100.0).round() as i32)
}

/// 行の高さの表示。書いていなければ「自動（値）」。
pub(crate) fn line_height_label(font: &FontSettings) -> String {
    let v = format!("{}", (font.line_height * 10.0).round() / 10.0);
    if font.line_height_explicit { v } else { format!("自動（{v}）") }
}

/// 書体の名前の一覧を、重なりを除いて大文字小文字を区別せずに並べる。"." で始まる（OS の内部の）ものは除く。
pub(crate) fn font_list(names: Vec<String>) -> Vec<String> {
    let mut list: Vec<String> = names.into_iter().filter(|n| !n.is_empty() && !n.starts_with('.')).collect();
    list.sort_by_key(|n| n.to_lowercase());
    list.dedup();
    list
}

/// query を含む書体（大文字小文字を区別しない）を max 個まで。
pub(crate) fn filter_fonts(list: &[String], query: &str, max: usize) -> Vec<String> {
    let q = query.trim().to_lowercase();
    list.iter().filter(|n| n.to_lowercase().contains(&q)).take(max).cloned().collect()
}

/// "#rrggbb" か "rrggbb"（前後の空白は無視）。
pub(crate) fn parse_hex_input(text: &str) -> Option<u32> {
    let t = text.trim();
    let hex = t.strip_prefix('#').unwrap_or(t);
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| u32::from_str_radix(hex, 16).ok()).flatten()
}

/// 設定パネルが WorkspaceView に頼むこと。
pub enum SettingsViewEvent {
    /// 設定ファイルを開く。
    OpenConfig(PathBuf),
}

impl EventEmitter<SettingsViewEvent> for SettingsView {}

/// どの書体の欄か。
#[derive(Clone, Copy, PartialEq, Eq)]
enum FontField {
    Ui,
    Mono,
}

pub struct SettingsView {
    focus: FocusHandle,
    scroll: ScrollHandle,
    opacity: Entity<SliderState>,
    accent_input: Entity<InputState>,
    ui_query: Entity<InputState>,
    mono_query: Entity<InputState>,
    /// 入っている書体（初めて描くときに一度だけ取る）。
    fonts: Option<Vec<String>>,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> SettingsView {
        let settings = cx.global::<ConfigState>().settings.clone();
        let opacity = cx.new(|_| {
            SliderState::new().min(OPACITY_MIN).max(OPACITY_MAX).step(0.01).default_value(settings.window.background_opacity)
        });
        let accent_input = cx.new(|cx| InputState::new(window, cx).placeholder("#rrggbb"));
        let ui_query = cx.new(|cx| InputState::new(window, cx).placeholder("書体を探す"));
        let mono_query = cx.new(|cx| InputState::new(window, cx).placeholder("書体を探す"));
        let mut subs = vec![
            cx.subscribe(&opacity, |_, _, ev: &SliderEvent, cx| match ev {
                SliderEvent::Change(v) => apply_edit(cx, PanelEdit::Opacity(v.end())),
                // 離したら待たずに書く。
                SliderEvent::Release(_) => crate::config_runtime::flush_pending(cx),
            }),
            cx.subscribe_in(&accent_input, window, |this, input, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let text = input.read(cx).value().to_string();
                    if let Some(c) = parse_hex_input(&text) {
                        apply_edit(cx, PanelEdit::Accent(Some(c)));
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                    }
                    this.notify_soon(cx);
                }
            }),
            cx.observe_global_in::<ConfigState>(window, |this, window, cx| this.sync_from_config(window, cx)),
        ];
        for input in [&ui_query, &mono_query] {
            subs.push(cx.subscribe(input, |_, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        SettingsView {
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            opacity,
            accent_input,
            ui_query,
            mono_query,
            fonts: None,
            _subs: subs,
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn notify_soon(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// 手で直した設定ファイルを読み直したときに、スライダーを合わせる。
    fn sync_from_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let v = cx.global::<ConfigState>().settings.window.background_opacity;
        if (self.opacity.read(cx).value().end() - v).abs() > 0.004 {
            self.opacity.update(cx, |s, cx| s.set_value(v, window, cx));
        }
        cx.notify();
    }

    fn query(&self, field: FontField) -> &Entity<InputState> {
        match field {
            FontField::Ui => &self.ui_query,
            FontField::Mono => &self.mono_query,
        }
    }
}

/// 小さなボタン（設定ファイルの知らせのボタンと同じ見た目）。
fn small_button(id: impl Into<ElementId>, label: impl Into<SharedString>, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(10.))
        .py(px(3.))
        .rounded(size::RADIUS_SM)
        .border_1()
        .border_color(theme.border_subtle)
        .cursor_pointer()
        .hover(|s| s.bg(theme.bg_hover))
        .child(label.into())
}

/// 選べるもの 1 つ（選んでいれば枠をアクセント色にする）。
fn chip(id: impl Into<ElementId>, label: impl Into<SharedString>, selected: bool, theme: &Theme) -> Stateful<Div> {
    let border = if selected { theme.accent } else { theme.border_subtle };
    div()
        .id(id)
        .px(px(10.))
        .py(px(4.))
        .rounded(size::RADIUS_SM)
        .border_1()
        .border_color(border)
        .when(selected, |d| d.text_color(theme.accent))
        .cursor_pointer()
        .hover(|s| s.bg(theme.bg_hover))
        .child(label.into())
}

/// オンとオフのスイッチ。
fn toggle(id: impl Into<ElementId>, on: bool, enabled: bool, theme: &Theme) -> Stateful<Div> {
    let track = if on { theme.accent } else { theme.border_subtle };
    div()
        .id(id)
        .w(px(34.))
        .h(px(18.))
        .rounded(px(9.))
        .bg(track)
        .p(px(2.))
        .flex()
        .when(on, |d| d.justify_end())
        .when(!enabled, |d| d.opacity(0.4))
        .when(enabled, |d| d.cursor_pointer())
        .child(div().size(px(14.)).rounded(px(7.)).bg(theme.fg_default))
}

/// 見出し。
fn heading(text: &'static str, theme: &Theme) -> Div {
    div().pt(px(18.)).pb(px(6.)).text_size(px(12.)).text_color(theme.fg_dim).child(text)
}

/// 項目 1 行（名前と、中身）。
fn row(label: &'static str, content: impl IntoElement, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(16.))
        .py(px(6.))
        .child(div().w(px(140.)).flex_shrink_0().pt(px(3.)).text_color(theme.fg_default).child(label))
        .child(div().flex_1().min_w_0().child(content))
}

impl SettingsView {
    fn render_appearance(&self, s: &Settings, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let themes = div().flex().flex_row().flex_wrap().gap(px(6.)).children(ThemeId::ALL.into_iter().map(|id| {
            chip(SharedString::from(format!("theme-{}", id.name())), id.label(), s.theme == id, theme)
                .on_click(cx.listener(move |_, _, _, cx| apply_edit(cx, PanelEdit::Theme(id))))
        }));
        let opacity = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .child(div().w(px(220.)).child(Slider::new(&self.opacity)))
            .child(div().text_color(theme.fg_dim).child(percent_label(s.window.background_opacity)));
        let blur = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .child(toggle("blur", s.window.blur, BLUR_SUPPORTED, theme).when(BLUR_SUPPORTED, |d| {
                let on = s.window.blur;
                d.on_click(cx.listener(move |_, _, _, cx| apply_edit(cx, PanelEdit::Blur(!on))))
            }))
            .when(!BLUR_SUPPORTED, |d| d.child(div().text_color(theme.fg_dim).child("この OS ではぼかしを使えません")));
        let presets = crate::themes::colors(s.theme).project_colors;
        let current = s.accent.unwrap_or(crate::themes::colors(s.theme).accent);
        let swatches = presets.into_iter().enumerate().map(|(i, c)| {
            let selected = s.accent == Some(c);
            div()
                .id(SharedString::from(format!("accent-{i}")))
                .size(px(22.))
                .rounded(px(11.))
                .bg(rgb(c))
                .border_2()
                .border_color(if selected { theme.fg_default } else { rgba(0x00000000) })
                .cursor_pointer()
                .on_click(cx.listener(move |_, _, _, cx| apply_edit(cx, PanelEdit::Accent(Some(c)))))
        });
        let accent = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .children(swatches)
            .child(div().w(px(110.)).child(Input::new(&self.accent_input).small()))
            .child(div().text_color(theme.fg_dim).child(format!("#{current:06x}")))
            .when(s.accent.is_some(), |d| {
                d.child(
                    small_button("accent-reset", "テーマの色に戻す", theme)
                        .on_click(cx.listener(|_, _, _, cx| apply_edit(cx, PanelEdit::Accent(None)))),
                )
            });
        div()
            .child(heading("外観", theme))
            .child(row("テーマ", themes, theme))
            .child(row("背景の不透明度", opacity, theme))
            .child(row("ぼかし", blur, theme))
            .child(row("アクセント色", accent, theme))
    }

    fn render_font_field(&mut self, field: FontField, s: &Settings, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let fonts = self.fonts.get_or_insert_with(|| font_list(window.text_system().all_font_names())).clone();
        let query = self.query(field).clone();
        let q = query.read(cx).value().to_string();
        let current = match field {
            FontField::Ui => s.font.ui.clone().unwrap_or_else(|| "OS の既定".to_string()),
            FontField::Mono => s.font.mono.clone(),
        };
        let matches = if q.trim().is_empty() { Vec::new() } else { filter_fonts(&fonts, &q, FONT_MATCHES) };
        let prefix = if field == FontField::Ui { "ui" } else { "mono" };
        let list = matches.into_iter().enumerate().map(|(i, name)| {
            let query = query.clone();
            let label = name.clone();
            div()
                .id(SharedString::from(format!("font-{prefix}-{i}")))
                .px(px(8.))
                .py(px(2.))
                .rounded(size::RADIUS_SM)
                .cursor_pointer()
                .hover(|st| st.bg(theme.bg_hover))
                .font_family(SharedString::from(label.clone()))
                .child(label)
                .on_click(cx.listener(move |_, _, window, cx| {
                    let edit = match field {
                        FontField::Ui => PanelEdit::UiFont(Some(name.clone())),
                        FontField::Mono => PanelEdit::MonoFont(name.clone()),
                    };
                    apply_edit(cx, edit);
                    query.update(cx, |q, cx| q.set_value("", window, cx));
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .child(div().min_w(px(160.)).child(current))
                    .child(div().w(px(180.)).child(Input::new(&query).small()))
                    .when(field == FontField::Ui && s.font.ui.is_some(), |d| {
                        d.child(
                            small_button("ui-font-reset", "OS の既定に戻す", theme)
                                .on_click(cx.listener(|_, _, _, cx| apply_edit(cx, PanelEdit::UiFont(None)))),
                        )
                    }),
            )
            .children(list)
    }

    fn render_text(&mut self, s: &Settings, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let size_now = s.font.size as u32;
        let stepper = |id: &'static str, label: &'static str, enabled: bool| {
            small_button(id, label, theme).when(!enabled, |d| d.opacity(0.4))
        };
        let font_size = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(stepper("size-down", "−", size_now > FONT_SIZE_MIN).on_click(cx.listener(move |_, _, _, cx| {
                apply_edit(cx, PanelEdit::FontSize(size_now.saturating_sub(1).max(FONT_SIZE_MIN)))
            })))
            .child(div().min_w(px(28.)).flex().justify_center().child(size_now.to_string()))
            .child(stepper("size-up", "+", size_now < FONT_SIZE_MAX).on_click(cx.listener(move |_, _, _, cx| {
                apply_edit(cx, PanelEdit::FontSize((size_now + 1).min(FONT_SIZE_MAX)))
            })));
        let lh = s.font.line_height;
        let explicit = s.font.line_height_explicit;
        let min = s.font.size;
        let line_height = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(stepper("lh-down", "−", lh > min).on_click(cx.listener(move |_, _, _, cx| {
                apply_edit(cx, PanelEdit::LineHeight(Some((lh - 1.0).max(min))))
            })))
            .child(div().min_w(px(80.)).flex().justify_center().child(line_height_label(&s.font)))
            .child(stepper("lh-up", "+", lh < LINE_HEIGHT_MAX).on_click(cx.listener(move |_, _, _, cx| {
                apply_edit(cx, PanelEdit::LineHeight(Some((lh + 1.0).min(LINE_HEIGHT_MAX))))
            })))
            .child(chip("lh-auto", "自動", !explicit, theme).on_click(cx.listener(|_, _, _, cx| apply_edit(cx, PanelEdit::LineHeight(None)))));
        div()
            .child(heading("文字", theme))
            .child(row("UI の書体", self.render_font_field(FontField::Ui, s, theme, window, cx), theme))
            .child(row("等幅の書体", self.render_font_field(FontField::Mono, s, theme, window, cx), theme))
            .child(row("文字の大きさ", font_size, theme))
            .child(row("行の高さ", line_height, theme))
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
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let state = cx.global::<ConfigState>().clone();
        let s = &state.settings;
        let appearance = self.render_appearance(s, &theme, cx);
        let text = self.render_text(s, &theme, window, cx);
        let cursor = self.render_cursor(s, &theme, cx);
        let open_config = state.path.clone().map(|path| {
            div().pt(px(24.)).child(
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
            .child(appearance)
            .child(text)
            .child(cursor)
            .children(open_config)
    }
}

#[cfg(test)]
mod tests {
    // super::* は gpui の test 属性も持ち込むので、名前を挙げる。
    use super::{filter_fonts, font_list, line_height_label, parse_hex_input, percent_label};

    #[test]
    fn opacity_percent_label() {
        assert_eq!(percent_label(0.62), "62%");
        assert_eq!(percent_label(1.0), "100%");
    }

    #[test]
    fn line_height_label_auto_and_explicit() {
        let mut f = fude_core::settings::Settings::default().font;
        assert_eq!(line_height_label(&f), "自動（18）");
        f.line_height = 22.5;
        f.line_height_explicit = true;
        assert_eq!(line_height_label(&f), "22.5");
    }

    #[test]
    fn font_list_dedups_and_sorts() {
        let names = vec!["B".to_string(), "a".into(), "B".into(), ".Hidden".into()];
        assert_eq!(font_list(names), ["a", "B"]);
    }

    #[test]
    fn font_filter_matches_case_insensitive() {
        let list: Vec<String> = ["Cascadia Code", "HackGen Console NF", "Segoe UI"].map(String::from).to_vec();
        assert_eq!(filter_fonts(&list, "hack", 8), ["HackGen Console NF"]);
        assert_eq!(filter_fonts(&list, "", 2).len(), 2);
    }

    #[test]
    fn hex_input_parses() {
        assert_eq!(parse_hex_input("#ff0080"), Some(0xff0080));
        assert_eq!(parse_hex_input(" FF0080 "), Some(0xff0080));
        assert_eq!(parse_hex_input("#ff008"), None);
        assert_eq!(parse_hex_input("red"), None);
    }
}
