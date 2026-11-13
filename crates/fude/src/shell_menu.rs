//! ＋の右クリックで出す、シェルの種類のメニュー。

use std::time::Instant;

use fude_core::motion::Fade;
use fude_core::workspace::ShellKind;
use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::hover::HoverKey;
use crate::theme::motion::{POPUP_IN, POPUP_OUT, ease_out_quint};
use crate::theme::{Theme, popup_shadow, size, ui_font};
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

const ITEM_H: Pixels = px(28.);
const MENU_MIN_W: Pixels = px(180.);

/// メニューの項目。Native が先頭、続けて WSL（Windows のときだけ）。
pub(crate) fn menu_items(distros: &[String], windows: bool) -> Vec<ShellKind> {
    let wsl = distros.iter().filter(|_| windows).map(|d| ShellKind::Wsl { distro: d.clone() });
    std::iter::once(ShellKind::Native).chain(wsl).collect()
}

/// 開いているメニューの状態。消える動きの途中も持ち続ける。
pub(crate) struct ShellMenu {
    position: Point<Pixels>,
    fade: Fade,
    closing: bool,
    focus: FocusHandle,
}

impl WorkspaceView {
    /// 押した位置を左上にしてメニューを出す。
    pub(crate) fn open_shell_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let mut fade = Fade::settled(0.0);
        fade.animate_to(1.0, POPUP_IN, ease_out_quint, Instant::now());
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.shell_menu = Some(ShellMenu { position, fade, closing: false, focus });
        cx.notify();
    }

    /// 消える動きを始める。終わったら描画のときに取り除く。
    pub(crate) fn close_shell_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.shell_menu.as_mut().filter(|m| !m.closing) {
            menu.closing = true;
            menu.fade.animate_to(0.0, POPUP_OUT, ease_out_quint, Instant::now());
            // メニューが持っていたフォーカスを、選択中のペインへ戻す。
            self.refocus_pane();
            cx.notify();
        }
    }

    pub(crate) fn render_shell_menu(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Option<Deferred> {
        let menu = self.shell_menu.as_ref()?;
        if menu.closing && !menu.fade.is_animating(now) {
            self.shell_menu = None;
            return None;
        }
        if menu.fade.is_animating(now) {
            window.request_animation_frame();
        }
        let theme = *cx.global::<Theme>();
        let (position, opacity, closing, focus) = (menu.position, menu.fade.value(now), menu.closing, menu.focus.clone());
        let items: Vec<AnyElement> = menu_items(&self.distros, cfg!(windows))
            .into_iter()
            .enumerate()
            .map(|(i, kind)| {
                let key = HoverKey::MenuItem(i);
                let bg = self.hover_bg(key, theme.bg_hover, now);
                let label = kind.label();
                self.hoverable(div().id(ElementId::Name(format!("shell-menu-{i}").into())), key, cx)
                    .h(ITEM_H)
                    .px(px(8.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .rounded(size::RADIUS_SM)
                    .bg(bg)
                    .child(icon(IconName::Terminal, theme.fg_dim))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.shell_menu.as_ref().is_some_and(|m| m.closing) {
                            return;
                        }
                        this.workspace.new_tab(kind.clone());
                        this.close_shell_menu(cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let body = div()
            .id("shell-menu")
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_shell_menu(cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_shell_menu(cx)))
            .opacity(opacity)
            .min_w(MENU_MIN_W)
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(size::RADIUS_MD)
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border_subtle)
            .shadow(popup_shadow())
            .font(ui_font())
            .text_size(px(13.))
            .text_color(theme.fg_default)
            .children(if closing { Vec::new() } else { items });
        Some(deferred(anchored().position(position).child(body)).with_priority(1))
    }
}

#[cfg(test)]
mod tests {
    use super::menu_items;
    use fude_core::workspace::ShellKind;

    #[test]
    fn menu_items_order() {
        let distros = vec!["Ubuntu".to_string(), "Debian".to_string()];
        assert_eq!(
            menu_items(&distros, true),
            vec![
                ShellKind::Native,
                ShellKind::Wsl { distro: "Ubuntu".into() },
                ShellKind::Wsl { distro: "Debian".into() },
            ]
        );
        assert_eq!(menu_items(&distros, false), vec![ShellKind::Native]);
    }
}
