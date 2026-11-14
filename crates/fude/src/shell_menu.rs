//! ＋の右クリックで出す、シェルの種類のメニュー。

use std::time::Instant;

use fude_core::workspace::ShellKind;
use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::hover::HoverKey;
use crate::popup::{ITEM_H, Popup, place};
use crate::theme::{Theme, size};
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

/// メニューの項目。Native が先頭、続けて WSL（Windows のときだけ）。
pub(crate) fn menu_items(distros: &[String], windows: bool) -> Vec<ShellKind> {
    let wsl = distros.iter().filter(|_| windows).map(|d| ShellKind::Wsl { distro: d.clone() });
    std::iter::once(ShellKind::Native).chain(wsl).collect()
}

impl WorkspaceView {
    /// 押した位置を左上にしてメニューを出す。
    pub(crate) fn open_shell_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.shell_menu = Some(Popup::open(position, window, cx));
        cx.notify();
    }

    /// 消える動きを始める。終わったら描画のときに取り除く。
    pub(crate) fn close_shell_menu(&mut self, cx: &mut Context<Self>) {
        if self.shell_menu.as_mut().is_some_and(|m| m.close(Instant::now())) {
            // メニューが持っていたフォーカスを、選択中のペインへ戻す。
            self.refocus_pane();
            cx.notify();
        }
    }

    pub(crate) fn render_shell_menu(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Option<Deferred> {
        let menu = self.shell_menu.as_ref()?;
        if menu.is_gone(now) {
            self.shell_menu = None;
            return None;
        }
        if menu.is_animating(now) {
            window.request_animation_frame();
        }
        let theme = *cx.global::<Theme>();
        let closing = menu.closing();
        let mut items: Vec<AnyElement> = menu_items(&self.distros, cfg!(windows))
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
                        if this.shell_menu.as_ref().is_some_and(|m| m.closing()) {
                            return;
                        }
                        this.workspace.new_tab(kind.clone());
                        this.close_shell_menu(cx);
                    }))
                    .into_any_element()
            })
            .collect();
        // 区切りの線と「Claude Code」（既定のシェルで新しいタブを開き、claude を起動する）。
        let key = HoverKey::MenuItem(items.len());
        let bg = self.hover_bg(key, theme.bg_hover, now);
        items.push(div().h(px(1.)).mx(px(4.)).my(px(4.)).bg(theme.border_subtle).into_any_element());
        items.push(
            self.hoverable(div().id("shell-menu-claude"), key, cx)
                .h(ITEM_H)
                .px(px(8.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .rounded(size::RADIUS_SM)
                .bg(bg)
                .child(icon(IconName::Sparkles, theme.fg_dim))
                .child("Claude Code")
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.shell_menu.as_ref().is_some_and(|m| m.closing()) {
                        return;
                    }
                    this.launch_claude(cx);
                    this.close_shell_menu(cx);
                }))
                .into_any_element(),
        );
        let body = menu
            .frame("shell-menu", &theme, now)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_shell_menu(cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_shell_menu(cx)))
            .children(if closing { Vec::new() } else { items });
        Some(place(menu.position(), body))
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
