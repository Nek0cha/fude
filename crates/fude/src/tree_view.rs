//! ファイルツリーの左パネル（設計書 §4.1、§4.3、§5.5）。レールの右に、メイン領域の上へ重ねて出す。

use std::collections::HashMap;
use std::time::Instant;

use fude_core::workspace::ProjectId;
use fude_fs::RelPath;
use fude_fs::watch::WatchBatch;
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::changes_menu::mark_style;
use crate::theme::{Theme, mono_font, size};
use crate::title_row::icon;
use crate::tree_model::{TreeModel, TreeRow};
use crate::workspace_view::WorkspaceView;

gpui_kit::actions!(tree, [ToggleTree, CloseTree]);

/// ツリーの開閉（⌘B、Windows は Ctrl+B。ターミナルの中では Ctrl+Shift+B。設計書 §10）と、Esc で閉じる。
pub fn bind_tree_keys(cx: &mut App) {
    // 開閉のキーは fude_core::keymap（workspace_view::bind_keys）で登録する。
    cx.bind_keys([KeyBinding::new("escape", CloseTree, Some("Tree"))]);
}

const ROW_H: Pixels = px(24.);
const INDENT: f32 = 12.;

/// 左パネルの状態。
pub(crate) struct TreeState {
    pub models: HashMap<ProjectId, TreeModel>,
    pub open: bool,
    pub filter: Option<Entity<InputState>>,
    pub focus: FocusHandle,
    /// 最後の描画で作った行（一覧の部品が範囲ごとに読む）。
    pub rows: Vec<TreeRow>,
    /// 選択中（最後にクリックした）行。
    pub selected: Option<RelPath>,
    /// 右クリックメニューと、その対象の行（None は何も無いところ）。
    pub menu: Option<(Option<TreeRow>, crate::popup::Popup)>,
    /// 新規や名前の変更で、名前を入れている途中。
    pub edit: Option<crate::tree_menu::TreeEdit>,
    _filter_sub: Option<Subscription>,
}

impl TreeState {
    pub fn new(cx: &mut App) -> TreeState {
        TreeState {
            models: HashMap::new(),
            open: false,
            filter: None,
            focus: cx.focus_handle(),
            rows: Vec::new(),
            selected: None,
            menu: None,
            edit: None,
            _filter_sub: None,
        }
    }
}

impl WorkspaceView {
    pub(crate) fn toggle_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.open {
            self.close_tree(window, cx);
            return;
        }
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        self.tree.open = true;
        if self.tree.filter.is_none() {
            let state = cx.new(|cx| InputState::new(window, cx));
            let sub = cx.subscribe(&state, |this, state, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    let query = state.read(cx).value().to_string();
                    this.set_tree_filter(query, cx);
                }
            });
            self.tree.filter = Some(state);
            self.tree._filter_sub = Some(sub);
        }
        if self.tree.models.entry(project).or_default().needs_load(&RelPath::root()) {
            self.load_tree_dir(project, RelPath::root(), cx);
        }
        window.focus(&self.tree.focus, cx);
        cx.notify();
    }

    pub(crate) fn close_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tree.open {
            return;
        }
        self.tree.open = false;
        self.restore_focus(window, cx);
        cx.notify();
    }

    /// dir の子を裏で読む。ProjectFs がまだ無ければ、用意できたとき（tree_live_ready）に読む。
    pub(crate) fn load_tree_dir(&mut self, project: ProjectId, dir: RelPath, cx: &mut Context<Self>) {
        let Some(fs) = self.live_fs(project) else { return };
        let task = cx.background_spawn({
            let dir = dir.clone();
            async move { fs.list_dir(&dir) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(entries) => this.tree.models.entry(project).or_default().set_children(&dir, entries),
                    Err(e) => eprintln!("フォルダを読めない: {}: {e}", dir.as_str()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// ProjectFs が用意できたときに呼ぶ。ツリーが開いていれば読み始める。
    pub(crate) fn tree_live_ready(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        if self.tree.open && self.tree.models.entry(project).or_default().needs_load(&RelPath::root()) {
            self.load_tree_dir(project, RelPath::root(), cx);
        }
    }

    fn set_tree_filter(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        let model = self.tree.models.entry(project).or_default();
        model.set_filter(&query, None);
        if model.filtering() && !model.has_file_list() {
            self.load_file_list(project, cx);
        }
        cx.notify();
    }

    fn load_file_list(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let Some(fs) = self.live_fs(project) else { return };
        let task = cx.background_spawn(async move { fs.walk_files() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(files) => {
                        let query = this.tree.filter.as_ref().map(|f| f.read(cx).value().to_string()).unwrap_or_default();
                        this.tree.models.entry(project).or_default().set_filter(&query, Some(&files));
                    }
                    Err(e) => eprintln!("ファイルの一覧を読めない: {e}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 監視の通知。読んだことのあるフォルダの中が変わったら読み直す。
    pub(crate) fn on_tree_changed(&mut self, project: ProjectId, batch: &WatchBatch, cx: &mut Context<Self>) {
        let Some(model) = self.tree.models.get_mut(&project) else { return };
        let dirs = model.invalidate(&batch.changed);
        let refetch = model.filtering();
        for dir in dirs {
            self.load_tree_dir(project, dir, cx);
        }
        if refetch {
            self.load_file_list(project, cx);
        }
    }

    fn click_row(&mut self, row: TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        self.tree.selected = Some(row.rel.clone());
        if row.is_dir {
            let model = self.tree.models.entry(project).or_default();
            if model.filtering() || model.changed_only() {
                return;
            }
            if model.toggle(&row.rel) && model.needs_load(&row.rel) {
                self.load_tree_dir(project, row.rel, cx);
            }
            cx.notify();
        } else {
            self.open_in_editor(project, row.rel.as_str().to_string(), None, None, window, cx);
        }
    }

    /// 描画のたびに呼ぶ。ツリーの出入りの動きを始める。
    pub(crate) fn observe_tree(&mut self, now: Instant) {
        use crate::theme::motion::{PANEL_IN, PANEL_OUT};
        let reduced = self.reduced_motion;
        if self.tree.open && !self.tree_presence.visible() {
            self.tree_presence.show(PANEL_IN, reduced, now);
        } else if !self.tree.open && self.tree_presence.visible() {
            self.tree_presence.hide(PANEL_OUT, reduced, now);
        }
    }

    pub(crate) fn render_tree(&mut self, cx: &mut Context<Self>, now: Instant) -> Option<Stateful<Div>> {
        // 閉じたあとも、消える動きの間は描き続ける。
        if !self.tree.open && self.tree_presence.is_gone(now) {
            return None;
        }
            let project = self.workspace.active_project()?.id;
        let theme = *cx.global::<Theme>();
        let status = self.live.get(&project).and_then(|l| l.status.clone());
        let model = self.tree.models.entry(project).or_default();
        self.tree.rows = model.rows(status.as_ref());
        let changed_only = model.changed_only();
        let count = self.tree.rows.len();

        let toggle_changed = div()
            .id("tree-changed-only")
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .rounded(size::RADIUS_SM)
            .when(changed_only, |d| d.bg(theme.bg_hover))
            .hover(|s| s.bg(theme.bg_hover))
            .child(icon(IconName::FileDiff, if changed_only { theme.fg_default } else { theme.fg_dim }))
            .on_click(cx.listener(move |this, _, _, cx| {
                let model = this.tree.models.entry(project).or_default();
                model.set_changed_only(!model.changed_only());
                cx.notify();
            }));
        let filter = self.tree.filter.clone().map(|state| {
            Input::new(&state).appearance(false).prefix(icon(IconName::Search, theme.fg_dim)).cleanable(true)
        });
        let list = uniform_list(
            "tree-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let theme = *cx.global::<Theme>();
                let selected = this.tree.selected.clone();
                this.tree.rows[range.start.min(this.tree.rows.len())..range.end.min(this.tree.rows.len())]
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(i, row)| {
                        let is_selected = selected.as_ref() == Some(&row.rel);
                        let chevron = if row.expanded { IconName::ChevronDown } else { IconName::ChevronRight };
                        let lead = if row.is_dir {
                            icon(chevron, theme.fg_dim).into_any_element()
                        } else {
                            crate::file_icons::file_icon(crate::file_icons::icon_for_file(&row.name)).into_any_element()
                        };
                        let deleted = row.mark == Some(fude_fs::git::GitMark::Deleted);
                        let mark = row.mark.map(|m| {
                            let (letter, color) = mark_style(m, &theme);
                            div().w(px(12.)).flex_shrink_0().font(mono_font()).text_size(px(11.)).text_color(color).child(letter)
                        });
                        let clicked = row.clone();
                        let menu_row = row.clone();
                        div()
                            .id(ElementId::Name(format!("tree-row-{}", range.start + i).into()))
                            .h(ROW_H)
                            .w_full()
                            .pl(px(8. + INDENT * row.depth as f32))
                            .pr(px(8.))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .rounded(size::RADIUS_SM)
                            .when(is_selected, |d| d.bg(theme.bg_hover))
                            .hover(|s| s.bg(theme.bg_hover))
                            .opacity(if deleted { 0.5 } else { 1.0 })
                            .child(lead)
                            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(row.name.clone()))
                            .children(mark)
                            .on_click(cx.listener(move |this, _, window, cx| this.click_row(clicked.clone(), window, cx)))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.open_tree_menu(Some(menu_row.clone()), ev.position, window, cx);
                                }),
                            )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1();
        let edit_row = self.tree.edit.as_ref().map(|e| {
            let kind_icon = match e.kind {
                crate::tree_menu::EditKind::NewFile => IconName::FilePlus,
                crate::tree_menu::EditKind::NewFolder => IconName::FolderPlus,
                crate::tree_menu::EditKind::Rename => IconName::Pencil,
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.))
                .px(px(4.))
                .rounded(size::RADIUS_SM)
                .border_1()
                .border_color(theme.accent)
                .on_action(cx.listener(|this, _: &gpui_kit::base::input::Escape, window, cx| this.cancel_tree_edit(window, cx)))
                .child(icon(kind_icon, theme.fg_dim))
                .child(div().flex_1().child(Input::new(&e.input).appearance(false)))
        });

        Some(
            div()
                .id("tree")
                .track_focus(&self.tree.focus)
                .key_context("Tree")
                .on_action(cx.listener(|this, _: &CloseTree, window, cx| this.close_tree(window, cx)))
                // 絞り込み欄は、消すものが無いときに Esc を親へ回す。
                .on_action(cx.listener(|this, _: &gpui_kit::base::input::Escape, window, cx| this.close_tree(window, cx)))
                // 重ねて出したとき、下のターミナルにクリックを渡さない。
                .occlude()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(6.))
                .text_size(px(13.))
                .text_color(theme.fg_default)
                .child(div().flex().flex_row().items_center().gap(px(4.)).children(filter.map(|f| div().flex_1().child(f))).child(toggle_changed))
                .children(edit_row)
                .child(
                    div()
                        .id("tree-body")
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, ev: &MouseDownEvent, window, cx| this.open_tree_menu(None, ev.position, window, cx)),
                        )
                        .child(list),
                ),
        )
    }
}
