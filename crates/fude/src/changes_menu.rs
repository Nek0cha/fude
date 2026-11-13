//! タイトル行の変更件数「● N」と、押すと出る変更したファイルの一覧（設計書 §4.3、§5.7）。

use std::time::Instant;

use fude_fs::git::{GitMark, GitStatus};
use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::file_view::initial_mode;
use crate::hover::HoverKey;
use crate::popup::{ITEM_H, Popup, place};
use crate::theme::{Theme, mono_font, size};
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

/// 一覧の並び。Git が追っているファイルの変更（M、A、D）を先に、未追跡（U）を後に。それぞれ名前順。
pub(crate) fn change_rows(status: &GitStatus) -> Vec<(String, GitMark)> {
    let mut rows: Vec<(String, GitMark)> = status.entries.iter().map(|(p, m)| (p.as_str().to_string(), *m)).collect();
    rows.sort_by(|a, b| (a.1 == GitMark::Untracked).cmp(&(b.1 == GitMark::Untracked)).then_with(|| a.0.cmp(&b.0)));
    rows
}

/// 件数の表示。999 を超えたら "999+"。
pub(crate) fn count_label(n: usize) -> String {
    if n > 999 { "999+".into() } else { n.to_string() }
}

/// Git の記号の文字と色（差分の色は「中身の色」。設計書 §6.1.1）。
pub(crate) fn mark_style(mark: GitMark, theme: &Theme) -> (&'static str, Rgba) {
    match mark {
        GitMark::Modified => ("M", theme.fg_default),
        GitMark::Untracked => ("U", theme.diff_add),
        GitMark::Added => ("A", theme.diff_add),
        GitMark::Deleted => ("D", theme.diff_del),
    }
}

/// 一覧のメニューの HoverKey の番号。
const MENU_HOVER: usize = 300;
/// 一覧に出す件数の上限。
const MENU_MAX: usize = 200;

impl WorkspaceView {
    /// タイトル行の「● N」。変更が 0 件なら何も出さない。
    pub(crate) fn render_changes_button(&self, cx: &mut Context<Self>, now: Instant) -> Option<Stateful<Div>> {
        let theme = *cx.global::<Theme>();
        let project = self.workspace.active_project()?.id;
        let n = self.live.get(&project)?.status.as_ref()?.len();
        if n == 0 {
            return None;
        }
        let key = HoverKey::Changes;
        Some(
            self.hoverable(div().id("changes"), key, cx)
                .h(px(24.))
                .px(px(8.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .flex_shrink_0()
                .rounded(px(12.))
                .bg(self.hover_bg(key, theme.bg_hover, now))
                .child(icon(IconName::FileDiff, theme.fg_dim).size(px(12.)))
                // 桁が変わっても隣が動かないよう、3 桁分の幅を取る。
                .child(div().min_w(px(26.)).font(mono_font()).text_size(px(12.)).text_color(theme.fg_default).child(count_label(n)))
                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    let pos = ev.position();
                    this.changes_menu = Some((project, Popup::open(point(pos.x - px(240.), px(40.)), window, cx)));
                    cx.notify();
                })),
        )
    }

    fn close_changes_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.changes_menu.as_mut().is_some_and(|(_, m)| m.close(Instant::now())) {
            self.restore_focus(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn render_changes_menu(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Option<Deferred> {
        let (project, menu) = self.changes_menu.as_ref()?;
        let project = *project;
        if menu.is_gone(now) {
            self.changes_menu = None;
            return None;
        }
        if menu.is_animating(now) {
            window.request_animation_frame();
        }
        let theme = *cx.global::<Theme>();
        let closing = menu.closing();
        let rows = self.live.get(&project).and_then(|l| l.status.as_ref()).map(change_rows).unwrap_or_default();
        let items: Vec<AnyElement> = rows
            .into_iter()
            .take(MENU_MAX)
            .enumerate()
            .map(|(i, (rel, mark))| {
                let key = HoverKey::MenuItem(MENU_HOVER + i);
                let (letter, color) = mark_style(mark, &theme);
                self.hoverable(div().id(ElementId::Name(format!("changes-{i}").into())), key, cx)
                    .h(ITEM_H)
                    .px(px(8.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .rounded(size::RADIUS_SM)
                    .bg(self.hover_bg(key, theme.bg_hover, now))
                    .child(div().w(px(10.)).font(mono_font()).text_color(color).child(letter))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(rel.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.changes_menu.as_ref().is_some_and(|(_, m)| m.closing()) {
                            return;
                        }
                        this.close_changes_menu(window, cx);
                        this.open_in_editor(project, rel.clone(), Some(initial_mode(Some(mark))), None, window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let body = menu
            .frame("changes-menu", &theme, now)
            .w(px(360.))
            .max_h(px(480.))
            .overflow_y_scroll()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_changes_menu(window, cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_changes_menu(window, cx)))
            .children(if closing { Vec::new() } else { items });
        Some(place(menu.position(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::{change_rows, count_label};
    use fude_fs::RelPath;
    use fude_fs::git::{GitMark, GitStatus};

    fn status(entries: &[(&str, GitMark)]) -> GitStatus {
        GitStatus { entries: entries.iter().map(|(p, m)| (RelPath::new(p).unwrap(), *m)).collect() }
    }

    #[test]
    fn rows_put_tracked_changes_first() {
        let s = status(&[
            ("a.rs", GitMark::Untracked),
            ("b.rs", GitMark::Modified),
            ("c.rs", GitMark::Deleted),
            ("d.rs", GitMark::Added),
            ("e.rs", GitMark::Untracked),
        ]);
        let rows: Vec<(String, GitMark)> = change_rows(&s);
        let names: Vec<&str> = rows.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, vec!["b.rs", "c.rs", "d.rs", "a.rs", "e.rs"]);
    }

    #[test]
    fn count_label_caps() {
        assert_eq!(count_label(3), "3");
        assert_eq!(count_label(999), "999");
        assert_eq!(count_label(1000), "999+");
    }
}
