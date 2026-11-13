//! ホーム画面（設計書 §4.1.2）。プロジェクトが 0 個のときに、メイン領域の代わりに出す。

use std::collections::HashSet;
use std::time::Instant;

use fude_core::recent::{HOME_RECENT, RecentList};
use fude_core::workspace::ProjectRoot;
use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::hover::HoverKey;
use crate::popup::{ITEM_H, Popup, place};
use crate::theme::{Theme, size};
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;
use crate::wsl_host::project_fs;

/// ホーム画面の「最近」の 1 行。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HomeItem {
    pub root: ProjectRoot,
    pub name: String,
    /// Local はフルパス、WSL は `WSL·<distro>  <path>`。
    pub place: String,
    pub missing: bool,
}

pub(crate) fn home_items(recent: &RecentList, missing: &HashSet<ProjectRoot>, n: usize) -> Vec<HomeItem> {
    recent
        .top(n)
        .iter()
        .map(|e| {
            let place = match &e.root {
                ProjectRoot::Local(p) => p.display().to_string(),
                ProjectRoot::Wsl { distro, path } => format!("WSL·{distro}  {path}"),
            };
            HomeItem {
                root: e.root.clone(),
                name: e.root.name(),
                place,
                missing: missing.iter().any(|m| m.same_place(&e.root)),
            }
        })
        .collect()
}

/// 「最近」の項目の右クリックメニューの HoverKey の番号（ほかのメニューと重ならないように）。
const MENU_HOVER: usize = 200;

impl WorkspaceView {
    /// ホーム画面に入ったときに呼ぶ。ローカルの項目のフォルダがあるかを裏で確かめる（WSL は起こさない）。
    pub(crate) fn check_missing_roots(&mut self, cx: &mut Context<Self>) {
        let roots: Vec<ProjectRoot> = self.recent.top(HOME_RECENT).iter().map(|e| e.root.clone()).collect();
        let task = cx.background_spawn(async move {
            roots.into_iter().filter(|r| r.local_path().is_some_and(|p| !p.is_dir())).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let missing = task.await;
            this.update(cx, |this, cx| {
                this.missing_roots.retain(|r| r.local_path().is_none());
                this.missing_roots.extend(missing);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 「最近」の項目を開く。ローカルはフォルダがあるか、WSL は fude serve で確かめてから開く。無ければ薄くする。
    fn open_recent(&mut self, root: ProjectRoot, cx: &mut Context<Self>) {
        if let Some(path) = root.local_path() {
            if path.is_dir() {
                self.open_project(root, None, cx);
            } else {
                self.missing_roots.insert(root);
            }
            cx.notify();
            return;
        }
        let hosts = self.wsl_hosts.clone();
        let probe = root.clone();
        let task = cx.background_spawn(async move {
            project_fs(&probe, &hosts).and_then(|fs| fs.metadata(&fude_fs::RelPath::root()))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(meta) if meta.is_dir => this.open_project(root, None, cx),
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        eprintln!("WSL のフォルダを確かめられない: {}: {e}", root.copy_text());
                    }
                    _ => {
                        this.missing_roots.insert(root);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_home_menu(&mut self, root: ProjectRoot, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.home_menu = Some((root, Popup::open(position, window, cx)));
        cx.notify();
    }

    fn close_home_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.home_menu.as_mut().is_some_and(|(_, m)| m.close(Instant::now())) {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(crate) fn render_home(&mut self, cx: &mut Context<Self>, now: Instant) -> Div {
        let theme = *cx.global::<Theme>();
        let can_resume = self.last_session.as_ref().is_some_and(|d| d.has_projects());
        let items = home_items(&self.recent, &self.missing_roots, HOME_RECENT);

        let open_folder = self
            .home_button("home-open", HoverKey::HomeOpenFolder, IconName::FolderOpen, "フォルダを開く", true, cx, now)
            .on_click(cx.listener(|this, _, _, cx| this.prompt_open_project(cx)));
        let resume = self
            .home_button("home-resume", HoverKey::HomeResume, IconName::RotateCcwClock, "前回の続き", can_resume, cx, now)
            .on_click(cx.listener(move |this, _, _, cx| {
                if can_resume {
                    this.resume_last_session(cx);
                }
            }));

        let rows: Vec<AnyElement> = items
            .into_iter()
            .enumerate()
            .map(|(i, item)| {
                let key = HoverKey::HomeItem(i);
                let root = item.root.clone();
                let menu_root = item.root.clone();
                self.hoverable(div().id(ElementId::Name(format!("home-item-{i}").into())), key, cx)
                    .h(px(32.))
                    .px(px(10.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .rounded(size::RADIUS_SM)
                    .bg(self.hover_bg(key, theme.bg_hover, now))
                    .opacity(if item.missing { 0.4 } else { 1.0 })
                    .child(div().flex_shrink_0().text_color(theme.fg_default).child(item.name))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_color(theme.fg_dim).child(item.place))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_recent(root.clone(), cx)))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            this.open_home_menu(menu_root.clone(), ev.position, window, cx)
                        }),
                    )
                    .into_any_element()
            })
            .collect();

        let mut column = div()
            .w(px(560.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(div().flex().flex_row().gap(px(12.)).child(open_folder).child(resume));
        if !rows.is_empty() {
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().px(px(10.)).pb(px(4.)).text_size(px(12.)).text_color(theme.fg_dim).child("最近"))
                    .children(rows),
            );
        }
        div().size_full().flex().justify_center().pt(px(120.)).px(px(24.)).text_size(px(13.)).child(column)
    }

    #[allow(clippy::too_many_arguments)]
    fn home_button(
        &self,
        id: &'static str,
        key: HoverKey,
        name: IconName,
        label: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        now: Instant,
    ) -> Stateful<Div> {
        let theme = *cx.global::<Theme>();
        let fg = if enabled { theme.fg_default } else { theme.fg_dim };
        let bg = if enabled { self.hover_bg(key, theme.bg_hover, now) } else { Rgba { a: 0.0, ..theme.bg_hover } };
        self.hoverable(div().id(id), key, cx)
            .h(px(36.))
            .px(px(14.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .rounded(size::RADIUS_MD)
            .border_1()
            .border_color(theme.border_subtle)
            .bg(bg)
            .text_color(fg)
            .child(icon(name, fg))
            .child(label)
    }

    pub(crate) fn render_home_menu(&mut self, window: &mut Window, cx: &mut Context<Self>, now: Instant) -> Option<Deferred> {
        let (root, menu) = self.home_menu.as_ref()?;
        let root = root.clone();
        if menu.is_gone(now) {
            self.home_menu = None;
            return None;
        }
        if menu.is_animating(now) {
            window.request_animation_frame();
        }
        let theme = *cx.global::<Theme>();
        let closing = menu.closing();
        let key = HoverKey::MenuItem(MENU_HOVER);
        let item = self
            .hoverable(div().id("home-menu-remove"), key, cx)
            .h(ITEM_H)
            .px(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .rounded(size::RADIUS_SM)
            .bg(self.hover_bg(key, theme.bg_hover, now))
            .child(icon(IconName::X, theme.fg_dim))
            .child("一覧から外す")
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.home_menu.as_ref().is_some_and(|(_, m)| m.closing()) {
                    return;
                }
                this.recent.remove(&root);
                this.save_recent(cx);
                this.close_home_menu(window, cx);
            }));
        let body = menu
            .frame("home-menu", &theme, now)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_home_menu(window, cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_home_menu(window, cx)))
            .children(if closing { None } else { Some(item) });
        Some(place(menu.position(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::home_items;
    use fude_core::recent::RecentList;
    use fude_core::workspace::ProjectRoot;
    use std::collections::HashSet;
    use std::path::PathBuf;

    #[test]
    fn home_items_show_place_and_missing() {
        let mut r = RecentList::default();
        r.touch(ProjectRoot::Local(PathBuf::from("/w/app")), 1);
        r.touch(ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x/fude".into() }, 2);
        let missing = HashSet::from([ProjectRoot::Local(PathBuf::from("/w/app"))]);
        let items = home_items(&r, &missing, 10);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "fude");
        assert_eq!(items[0].place, "WSL·Ubuntu  /home/x/fude");
        assert!(!items[0].missing);
        assert_eq!(items[1].name, "app");
        assert_eq!(items[1].place, "/w/app");
        assert!(items[1].missing);
        assert_eq!(home_items(&r, &missing, 1).len(), 1);
    }
}
