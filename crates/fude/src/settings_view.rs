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
use crate::hook_targets::{HookOp, HookTarget, TargetState};
use fude_core::claude_settings::HookStatus;
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
    /// Claude Code の hook の登録先と、その状態（設計書 §5.4）。
    targets: Vec<(HookTarget, TargetState)>,
    /// WSL 側の fude の置き場所。
    wsl_bin: String,
    _subs: Vec<Subscription>,
}

/// 登録先の状態の文と、押せるボタン（何をするか、文言）。
pub(crate) fn target_row(state: &TargetState) -> (String, Vec<(HookOp, &'static str)>) {
    match state {
        TargetState::Unchecked => ("まだ調べていません".into(), vec![(HookOp::Check, "調べる")]),
        TargetState::Checking => ("調べています…".into(), vec![]),
        TargetState::Known(HookStatus::Registered) => {
            ("登録済み".into(), vec![(HookOp::Install, "登録し直す"), (HookOp::Uninstall, "外す")])
        }
        TargetState::Known(HookStatus::NotRegistered) => ("まだ登録していません".into(), vec![(HookOp::Install, "登録する")]),
        TargetState::Known(HookStatus::Outdated) => {
            ("登録し直してください".into(), vec![(HookOp::Install, "登録し直す"), (HookOp::Uninstall, "外す")])
        }
        TargetState::Known(HookStatus::Unreadable(r)) => (format!("読めません: {r}"), vec![(HookOp::Check, "調べ直す")]),
        TargetState::NotInstalled => {
            ("まだ登録していません（登録するときに fude を WSL に置きます）".into(), vec![(HookOp::Install, "登録する")])
        }
    }
}

/// パネルを開き直したときに調べ直す登録先（設計書 §5.4）。この PC と、一度調べた WSL。
/// まだ調べていない WSL は起こさないため、調べている最中のものは二重に走らせないため除く。
pub(crate) fn rows_to_recheck(targets: &[(HookTarget, TargetState)]) -> Vec<HookTarget> {
    targets
        .iter()
        .filter(|(t, s)| match s {
            TargetState::Checking => false,
            TargetState::Unchecked => *t == HookTarget::ThisPc,
            TargetState::Known(_) | TargetState::NotInstalled => true,
        })
        .map(|(t, _)| t.clone())
        .collect()
}

/// 登録先の名前。
fn target_label(target: &HookTarget) -> String {
    match target {
        HookTarget::ThisPc if cfg!(target_os = "macos") => "この Mac".to_string(),
        HookTarget::ThisPc => "Windows（この PC）".to_string(),
        HookTarget::Wsl(d) => format!("WSL: {d}"),
    }
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
        #[cfg(windows)]
        let wsl_bin = fude_fs::wsl::default_bin(&|k| std::env::var(k).ok());
        #[cfg(not(windows))]
        let wsl_bin = String::new();
        let mut view = SettingsView {
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            opacity,
            accent_input,
            ui_query,
            mono_query,
            fonts: None,
            targets: vec![(HookTarget::ThisPc, TargetState::Unchecked)],
            wsl_bin,
            _subs: subs,
        };
        view.run_target(HookTarget::ThisPc, HookOp::Check, cx);
        view.reload_distros(cx);
        view
    }

    /// パネルを開き直した。hook の状態を調べ直す。
    pub fn reopened(&mut self, cx: &mut Context<Self>) {
        for target in rows_to_recheck(&self.targets) {
            self.run_target(target, HookOp::Check, cx);
        }
    }

    /// target に op を裏で行い、終わったら状態を差し替える。
    fn run_target(&mut self, target: HookTarget, op: HookOp, cx: &mut Context<Self>) {
        let Some(slot) = self.targets.iter_mut().find(|(t, _)| *t == target) else { return };
        if slot.1 == TargetState::Checking {
            return;
        }
        slot.1 = TargetState::Checking;
        cx.notify();
        let bin = self.wsl_bin.clone();
        let job_target = target.clone();
        let task = cx.background_spawn(async move { crate::hook_targets::run(&job_target, op, &bin) });
        cx.spawn(async move |this, cx| {
            let state = task.await;
            this.update(cx, |this, cx| {
                if let Some(slot) = this.targets.iter_mut().find(|(t, _)| *t == target) {
                    slot.1 = state;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// WSL のディストリビューションの一覧を読み直す（Windows だけ）。調べた状態は、同じ名前なら残す。
    fn reload_distros(&mut self, cx: &mut Context<Self>) {
        if !cfg!(windows) {
            return;
        }
        let task = cx.background_spawn(async { fude_term::list_distros() });
        cx.spawn(async move |this, cx| {
            let Ok(distros) = task.await else { return };
            this.update(cx, |this, cx| {
                let old = std::mem::take(&mut this.targets);
                let state_of = |t: &HookTarget| old.iter().find(|(o, _)| o == t).map(|(_, s)| s.clone());
                let mut targets = vec![(HookTarget::ThisPc, state_of(&HookTarget::ThisPc).unwrap_or(TargetState::Unchecked))];
                for d in distros {
                    let t = HookTarget::Wsl(d);
                    let s = state_of(&t).unwrap_or(TargetState::Unchecked);
                    targets.push((t, s));
                }
                this.targets = targets;
                cx.notify();
            })
            .ok();
        })
        .detach();
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

    fn render_hooks(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let rows = self.targets.iter().enumerate().map(|(i, (target, state))| {
            let (text, actions) = target_row(state);
            let color = match state {
                TargetState::Known(HookStatus::Registered) => theme.fg_default,
                TargetState::Known(HookStatus::Outdated | HookStatus::Unreadable(_)) => theme.warn,
                _ => theme.fg_dim,
            };
            let buttons = actions.into_iter().enumerate().map(|(j, (op, label))| {
                let target = target.clone();
                small_button(SharedString::from(format!("hook-{i}-{j}")), label, theme)
                    .on_click(cx.listener(move |this, _, _, cx| this.run_target(target.clone(), op, cx)))
            });
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .py(px(4.))
                .child(div().w(px(160.)).flex_shrink_0().child(target_label(target)))
                .child(div().flex_1().min_w_0().text_color(color).child(text))
                .children(buttons)
        });
        let dev_note = std::env::var("FUDE_INSTANCE").ok().filter(|v| !v.is_empty()).map(|_| {
            div()
                .pb(px(6.))
                .text_color(theme.warn)
                .child("開発用のインスタンスです。登録すると普段の fude の hook を置き換えます")
        });
        div()
            .child(heading("Claude Code の hook", theme))
            .child(
                div()
                    .pb(px(6.))
                    .text_color(theme.fg_dim)
                    .child("ボタンを押したときだけ、Claude Code の settings.json に fude の hook を書きます。ほかの設定は残します。"),
            )
            .children(dev_note)
            .children(rows)
            .when(cfg!(windows), |d| {
                d.child(
                    div().pt(px(6.)).flex().child(
                        small_button("hook-reload-distros", "WSL の一覧を読み直す", theme)
                            .on_click(cx.listener(|this, _, _, cx| this.reload_distros(cx))),
                    ),
                )
            })
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
        let hooks = self.render_hooks(&theme, cx);
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
            .child(appearance)
            .child(text)
            .child(cursor)
            .child(hooks)
            .children(open_config)
    }
}

#[cfg(test)]
mod tests {
    // super::* は gpui の test 属性も持ち込むので、名前を挙げる。
    use super::{filter_fonts, font_list, line_height_label, parse_hex_input, percent_label, rows_to_recheck, target_row};
    use crate::hook_targets::{HookOp, HookTarget, TargetState};
    use fude_core::claude_settings::HookStatus;

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

    #[test]
    fn row_texts_for_each_state() {
        let ops = |st: &TargetState| target_row(st).1.into_iter().map(|(op, label)| (op, label)).collect::<Vec<_>>();
        assert_eq!(target_row(&TargetState::Unchecked).0, "まだ調べていません");
        assert_eq!(ops(&TargetState::Unchecked), [(HookOp::Check, "調べる")]);
        assert_eq!(target_row(&TargetState::Checking).0, "調べています…");
        assert!(ops(&TargetState::Checking).is_empty());
        let reg = TargetState::Known(HookStatus::Registered);
        assert_eq!(target_row(&reg).0, "登録済み");
        assert_eq!(ops(&reg), [(HookOp::Install, "登録し直す"), (HookOp::Uninstall, "外す")]);
        let not = TargetState::Known(HookStatus::NotRegistered);
        assert_eq!(target_row(&not).0, "まだ登録していません");
        assert_eq!(ops(&not), [(HookOp::Install, "登録する")]);
        let old = TargetState::Known(HookStatus::Outdated);
        assert_eq!(target_row(&old).0, "登録し直してください");
        assert_eq!(ops(&old), [(HookOp::Install, "登録し直す"), (HookOp::Uninstall, "外す")]);
        let bad = TargetState::Known(HookStatus::Unreadable("x".into()));
        assert_eq!(target_row(&bad).0, "読めません: x");
        assert_eq!(ops(&bad), [(HookOp::Check, "調べ直す")]);
        assert_eq!(target_row(&TargetState::NotInstalled).0, "まだ登録していません（登録するときに fude を WSL に置きます）");
        assert_eq!(ops(&TargetState::NotInstalled), [(HookOp::Install, "登録する")]);
    }

    #[test]
    fn reopening_rechecks_this_pc_and_checked_wsl() {
        let targets = vec![
            (HookTarget::ThisPc, TargetState::Known(HookStatus::NotRegistered)),
            (HookTarget::Wsl("A".into()), TargetState::Unchecked),
            (HookTarget::Wsl("B".into()), TargetState::Known(HookStatus::Registered)),
            (HookTarget::Wsl("C".into()), TargetState::NotInstalled),
            (HookTarget::Wsl("D".into()), TargetState::Checking),
        ];
        assert_eq!(
            rows_to_recheck(&targets),
            [HookTarget::ThisPc, HookTarget::Wsl("B".into()), HookTarget::Wsl("C".into())]
        );
    }
}
