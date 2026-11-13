//! 確認ダイアログ。未保存の変更、ツリーの削除、動いているターミナルを閉じるときに出す（設計書 §4.3、§5.12）。
//! Enter は最初の Primary か Danger、Esc と外側のクリックは Cancel。出ている間は、ほかの操作を受け付けない。

use std::io;
use std::time::Instant;

use gpui_kit::*;

use crate::popup::Popup;
use crate::theme::{Theme, size};
use crate::workspace_view::WorkspaceView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonRole {
    Primary,
    Danger,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DialogButton {
    pub label: &'static str,
    pub role: ButtonRole,
}

pub(crate) type OnChoose = Box<dyn FnOnce(usize, &mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>)>;

pub(crate) struct Dialog {
    pub message: String,
    pub buttons: Vec<DialogButton>,
    /// 押したボタンの位置を受け取る。Cancel のボタンが無いダイアログで Esc を押したときは呼ばない。
    pub on_choose: OnChoose,
}

/// Enter で選ぶボタン。ボタンが 1 つだけなら、それを選ぶ。
pub(crate) fn default_index(buttons: &[DialogButton]) -> Option<usize> {
    buttons
        .iter()
        .position(|b| matches!(b.role, ButtonRole::Primary | ButtonRole::Danger))
        .or_else(|| (buttons.len() == 1).then_some(0))
}

/// 知らせるだけのダイアログのボタン（Enter でも Esc でも閉じる）。
pub(crate) fn ok_dialog_buttons() -> Vec<DialogButton> {
    vec![DialogButton { label: "OK", role: ButtonRole::Cancel }]
}

/// ファイル操作の失敗を知らせる文。what は「保存できません」など、name は対象の名前。
pub(crate) fn error_text(what: &str, name: &str, e: &io::Error) -> String {
    let reason = if e.kind() == io::ErrorKind::AlreadyExists {
        "同じ名前のものがすでにあります".to_string()
    } else {
        e.to_string()
    };
    format!("{what}: {name}\n{reason}")
}

/// Esc と外側のクリックで選ぶボタン。
pub(crate) fn cancel_index(buttons: &[DialogButton]) -> Option<usize> {
    buttons.iter().position(|b| b.role == ButtonRole::Cancel)
}

/// 選んだあと、ターミナルへフォーカスを戻すか。選んだ処理がフォーカスを動かした（still_on_workspace が
/// false。閉じたタブの隣のエディタなど）ときは、その場所を保つ。
pub(crate) fn refocus_after_choose(still_on_workspace: bool) -> bool {
    still_on_workspace
}

/// 出ているダイアログ。消える動きの間も持つ（on_choose は選んだ時点で取り出す）。
pub(crate) struct OpenDialog {
    message: String,
    buttons: Vec<DialogButton>,
    on_choose: Option<OnChoose>,
    popup: Popup,
}

impl WorkspaceView {
    pub(crate) fn show_dialog(&mut self, dialog: Dialog, window: &mut Window, cx: &mut Context<Self>) {
        let Dialog { message, buttons, on_choose } = dialog;
        let popup = Popup::open(point(px(0.), px(0.)), window, cx);
        self.dialog = Some(OpenDialog { message, buttons, on_choose: Some(on_choose), popup });
        cx.notify();
    }

    /// 失敗を知らせる（OK だけのダイアログ）。
    pub(crate) fn show_error(&mut self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = Dialog { message, buttons: ok_dialog_buttons(), on_choose: Box::new(|_, _, _, _| {}) };
        self.show_dialog(dialog, window, cx);
    }

    /// 選ぶ前の（消える動きに入っていない）ダイアログがあるか。
    pub(crate) fn dialog_open(&self) -> bool {
        self.dialog.as_ref().is_some_and(|d| d.on_choose.is_some())
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.dialog.as_mut() else { return };
        let Some(on_choose) = d.on_choose.take() else { return };
        d.popup.close(Instant::now());
        window.focus(&self.focus, cx);
        on_choose(index, self, window, cx);
        if refocus_after_choose(self.focus.is_focused(window)) {
            self.refocus_pane();
        }
        cx.notify();
    }

    pub(crate) fn render_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Option<Deferred> {
        let d = self.dialog.as_ref()?;
        if d.popup.is_gone(now) {
            self.dialog = None;
            return None;
        }
        if d.popup.is_animating(now) {
            window.request_animation_frame();
        }
        let theme = *cx.global::<Theme>();
        let buttons: Vec<AnyElement> = d
            .buttons
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (bg, fg) = match b.role {
                    ButtonRole::Primary => (theme.accent, rgb(0x14141c)),
                    ButtonRole::Danger => (Rgba { a: 0.22, ..theme.danger }, theme.danger),
                    ButtonRole::Cancel => (theme.bg_hover, theme.fg_default),
                };
                div()
                    .id(ElementId::Name(format!("dialog-button-{i}").into()))
                    .h(px(30.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .rounded(size::RADIUS_SM)
                    .bg(bg)
                    .text_color(fg)
                    .child(b.label)
                    .on_click(cx.listener(move |this, _, window, cx| this.choose(i, window, cx)))
                    .into_any_element()
            })
            .collect();
        let (enter, cancel) = (default_index(&d.buttons), cancel_index(&d.buttons));
        let body = d
            .popup
            .frame("dialog", &theme, now)
            .w(px(380.))
            .p(px(18.))
            .gap(px(18.))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "enter" => {
                        if let Some(i) = enter {
                            this.choose(i, window, cx);
                        }
                    }
                    "escape" => {
                        if let Some(i) = cancel {
                            this.choose(i, window, cx);
                        }
                    }
                    _ => {}
                }
                cx.stop_propagation();
            }))
            .child(div().text_color(theme.fg_default).child(d.message.clone()))
            .child(div().flex().flex_row().justify_end().gap(px(8.)).children(buttons));
        // 全面を覆う透明な層で、外側のクリックを受けて取り消す（ほかの部品には届かせない）。
        let overlay = div()
            .id("dialog-overlay")
            .absolute()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if let Some(i) = cancel {
                        this.choose(i, window, cx);
                    }
                }),
            )
            .child(div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(body));
        Some(deferred(overlay).with_priority(2))
    }
}

#[cfg(test)]
mod tests {
    use super::{ButtonRole, DialogButton, cancel_index, default_index, error_text, ok_dialog_buttons, refocus_after_choose};

    #[test]
    fn choice_that_moved_focus_keeps_it() {
        // 選んだ処理がフォーカスを動かした（隣のエディタのタブなど）なら、ターミナルへ戻さない。
        assert!(!refocus_after_choose(false));
        // 動かさなかった（キャンセルなど）なら、ターミナルへ戻す。
        assert!(refocus_after_choose(true));
    }

    fn b(label: &'static str, role: ButtonRole) -> DialogButton {
        DialogButton { label, role }
    }

    #[test]
    fn dialog_keys() {
        let save = [b("保存", ButtonRole::Primary), b("保存しない", ButtonRole::Danger), b("キャンセル", ButtonRole::Cancel)];
        assert_eq!(default_index(&save), Some(0));
        assert_eq!(cancel_index(&save), Some(2));
        let close = [b("キャンセル", ButtonRole::Cancel), b("閉じる", ButtonRole::Danger)];
        assert_eq!(default_index(&close), Some(1));
        assert_eq!(cancel_index(&close), Some(0));
        assert_eq!(cancel_index(&[b("OK", ButtonRole::Primary)]), None);
    }

    #[test]
    fn ok_dialog_closes_with_enter_and_esc() {
        let ok = ok_dialog_buttons();
        assert_eq!(ok.len(), 1);
        assert_eq!(default_index(&ok), Some(0));
        assert_eq!(cancel_index(&ok), Some(0));
    }

    #[test]
    fn error_text_for_io_errors() {
        let exists = std::io::Error::new(std::io::ErrorKind::AlreadyExists, "x");
        assert_eq!(error_text("名前を変えられません", "b.rs", &exists), "名前を変えられません: b.rs
同じ名前のものがすでにあります");
        let other = std::io::Error::other("ディスクがいっぱい");
        assert_eq!(error_text("保存できません", "a.rs", &other), "保存できません: a.rs
ディスクがいっぱい");
    }
}
