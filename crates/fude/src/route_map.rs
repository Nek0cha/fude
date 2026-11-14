//! 路線図（設計書 §4.1、§5.8）。プランのタスク、無ければ Claude Code のタスクを駅のように縦に並べる。
//!
//! プランのファイルは gitignore されていることが多く監視できないため、作業中のプロジェクトだけを PLAN_POLL ごとに
//! `ProjectFs::plan_files` で確かめる（印が変わったときだけ中身を読む）。

use std::io;
use std::time::{Duration, Instant};

use fude_core::location::FileLocation;
use fude_core::plan::{Route, StationState, plan_is_recent, plan_route, todo_route};
use fude_core::workspace::{ProjectId, ProjectRoot};
use fude_fs::plans::{PlanFetch, PlanStamp};
use gpui_kit::assets::IconName;
use gpui_kit::*;

use crate::theme::{Theme, mono_font};
use crate::title_row::icon;
use crate::workspace_view::WorkspaceView;

pub(crate) const PLAN_POLL: Duration = Duration::from_secs(2);
/// 駅 1 つの高さ。
pub(crate) const STATION_H: f32 = 28.0;
/// 開いたときと、折りたたんだときの幅。
const ROUTE_W: f32 = 180.0;
const ROUTE_COLLAPSED_W: f32 = 28.0;
/// 印の列の幅と、輪の大きさ。
const MARK_W: f32 = 28.0;
const RING: f32 = 10.0;
const HEADER_H: f32 = 32.0;

/// プロジェクトのプランの最後の読み取り。
#[derive(Default)]
pub(crate) struct PlanCache {
    pub stamp: Option<PlanStamp>,
    pub route: Option<Route>,
    /// 台帳も印も無く、更新が最近なので出しているだけ（期間を過ぎたら消す）。
    pub recent_only: bool,
}

/// 今の UNIX 時刻（ミリ秒）。
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// 同じプラン（同じ rel と更新時刻）の台帳だけが消えた。Superpowers はプランを終えると台帳を消すため、
/// 終わったプランとみなして隠す（台帳なしで作り直すと、終わったプランが 0/N に見えてしまう）。
pub(crate) fn ledger_removed(prev: Option<&PlanStamp>, new: &PlanStamp) -> bool {
    prev.is_some_and(|p| {
        p.rel == new.rel && p.plan_mtime_ms == new.plan_mtime_ms && p.ledger_mtime_ms.is_some() && new.ledger_mtime_ms.is_none()
    })
}

/// 幅の目標。route が無ければ 0、折りたたみなら ROUTE_COLLAPSED_W、それ以外は ROUTE_W。
pub(crate) fn route_width(has_route: bool, collapsed: bool) -> f32 {
    match (has_route, collapsed) {
        (false, _) => 0.0,
        (true, true) => ROUTE_COLLAPSED_W,
        (true, false) => ROUTE_W,
    }
}

/// 輪の印の上端（駅の並びの上端から）。index は小数（動きの途中）。
pub(crate) fn ring_y(index: f32) -> f32 {
    index * STATION_H + (STATION_H - RING) / 2.0
}

/// プランのファイルの場所。
pub(crate) fn plan_location(root: &ProjectRoot, rel: &str) -> FileLocation {
    match root {
        ProjectRoot::Local(base) => FileLocation::Local(rel.split('/').fold(base.clone(), |p, part| p.join(part))),
        ProjectRoot::Wsl { distro, path } => {
            FileLocation::Wsl { distro: distro.clone(), path: format!("{}/{rel}", path.trim_end_matches('/')) }
        }
    }
}

impl WorkspaceView {
    /// プランがあればプラン、無ければそのプロジェクトのペインの Todo。
    pub(crate) fn project_route(&self, project: ProjectId) -> Option<Route> {
        if let Some(route) = self.plans.get(&project).and_then(|c| c.route.clone()) {
            return Some(route);
        }
        let p = self.workspace.projects().iter().find(|p| p.id == project)?;
        let panes = p.tabs.iter().flat_map(|t| &t.panes).map(|pane| pane.id.raw());
        todo_route(self.agents.latest_todos(panes)?)
    }

    pub(crate) fn toggle_route_map(&mut self, cx: &mut Context<Self>) {
        self.route_collapsed = !self.route_collapsed;
        cx.notify();
    }

    /// 繰り返しの確かめを始める（WorkspaceView を作ったときに 1 回）。
    pub(crate) fn start_plan_poll(&mut self, cx: &mut Context<Self>) {
        self.plan_poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PLAN_POLL).await;
                if this.update(cx, |this, cx| this.check_plan(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// 作業中のプロジェクトのプランを裏で確かめる。前の確かめが終わっていなければ何もしない。
    pub(crate) fn check_plan(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.workspace.active_project().map(|p| p.id) else { return };
        let Some(fs) = self.live_fs(project) else { return };
        if !self.plan_checking.insert(project) {
            return;
        }
        let known = self.plans.get(&project).and_then(|c| c.stamp.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { fs.plan_files(known.as_ref()) }).await;
            this.update(cx, |this, cx| {
                this.plan_checking.remove(&project);
                this.apply_plan_fetch(project, result, cx);
            })
            .ok();
        })
        .detach();
    }

    fn apply_plan_fetch(&mut self, project: ProjectId, result: io::Result<PlanFetch>, cx: &mut Context<Self>) {
        match result {
            Ok(PlanFetch::Changed { stamp, plan, ledger }) => {
                let prev = self.plans.get(&project).and_then(|c| c.stamp.as_ref());
                if ledger_removed(prev, &stamp) {
                    self.plans.insert(project, PlanCache { stamp: Some(stamp), route: None, recent_only: false });
                    cx.notify();
                    return;
                }
                let recent = plan_is_recent(stamp.plan_mtime_ms, now_ms());
                let started = plan_route(stamp.rel.as_str(), &plan, ledger.as_deref(), false);
                let recent_only = started.is_none() && recent;
                let route = if recent_only { plan_route(stamp.rel.as_str(), &plan, None, true) } else { started };
                self.plans.insert(project, PlanCache { stamp: Some(stamp), route, recent_only });
                cx.notify();
            }
            // 変わっていなくても、最近だから出していたプランは、期間を過ぎたら消す。
            Ok(PlanFetch::Unchanged) => {
                if let Some(cache) = self.plans.get_mut(&project)
                    && cache.recent_only
                    && cache.stamp.as_ref().is_some_and(|s| !plan_is_recent(s.plan_mtime_ms, now_ms()))
                {
                    cache.route = None;
                    cache.recent_only = false;
                    cx.notify();
                }
            }
            Ok(PlanFetch::None) => {
                if self.plans.remove(&project).is_some_and(|c| c.route.is_some()) {
                    cx.notify();
                }
            }
            // 変わっていない、または読めない（次にまたやる）。
            Err(_) => {}
        }
    }

    /// 幅と輪の動きを今の路線図に合わせる。プロジェクトが変わったら、すぐプランを確かめる。
    pub(crate) fn observe_route_map(&mut self, window: &mut Window, now: Instant, cx: &mut Context<Self>) {
        use crate::theme::motion::{PANEL_IN, PANEL_OUT, REDUCED, RING_MOVE, ease_in_out, ease_out_quint};
        let open: std::collections::HashSet<ProjectId> = self.workspace.projects().iter().map(|p| p.id).collect();
        self.plans.retain(|id, _| open.contains(id));
        let active = self.workspace.active_project().map(|p| p.id);
        if active != self.route_project {
            self.route_project = active;
            // 別のプロジェクトの輪の位置から動かさない。
            self.ring_shown = false;
            self.check_plan(cx);
        }
        let route = active.and_then(|id| self.project_route(id));
        if route.is_some() {
            self.route_last = route.clone();
        }
        let target = route_width(route.is_some(), self.route_collapsed);
        let duration = match (self.reduced_motion, target > self.route_w.target()) {
            (true, _) => REDUCED,
            (false, true) => PANEL_IN,
            (false, false) => PANEL_OUT,
        };
        self.route_w.animate_to(target, duration, ease_out_quint, now);
        let current = route.as_ref().and_then(Route::current).map(|i| i as f32);
        match current {
            // 初めて出たとき、動きを減らす設定のときは、動かさずに置く。
            Some(i) if self.ring_shown && !self.reduced_motion => self.ring.animate_to(i, RING_MOVE, ease_in_out, now),
            Some(i) => self.ring = fude_core::motion::Fade::settled(i),
            None => {}
        }
        self.ring_shown = current.is_some();
        if self.route_w.is_animating(now) || self.ring.is_animating(now) {
            window.request_animation_frame();
        }
    }

    /// 今の幅（動きの途中を含む）。
    pub(crate) fn route_map_width(&self, now: Instant) -> f32 {
        self.route_w.value(now)
    }

    pub(crate) fn render_route_map(&mut self, now: Instant, cx: &mut Context<Self>) -> Option<AnyElement> {
        let width = self.route_w.value(now);
        let project = self.workspace.active_project()?;
        let root = project.root.clone();
        if width < 0.5 {
            self.route_last = None;
            return None;
        }
        // 消えていく間（プランも Todo も無くなった直後）は、最後に出したものを描く。
        let route = self.project_route(project.id).or_else(|| self.route_last.clone())?;
        let theme = *cx.global::<Theme>();
        let collapsed = self.route_collapsed;
        let inner = if collapsed {
            div()
                .id("route-collapsed")
                .w(px(ROUTE_COLLAPSED_W))
                .h(px(HEADER_H))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(IconName::ListTodo, theme.fg_dim))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_route_map(cx)))
                .into_any_element()
        } else {
            self.render_route_body(&route, root, &theme, now, cx)
        };
        Some(
            div()
                .w(px(width))
                .h_full()
                .flex_shrink_0()
                .overflow_hidden()
                .border_r_1()
                .border_color(theme.border_subtle)
                .child(inner)
                .into_any_element(),
        )
    }

    fn render_route_body(&self, route: &Route, root: ProjectRoot, theme: &Theme, now: Instant, cx: &mut Context<Self>) -> AnyElement {
        let (done, total) = route.progress();
        let header = div()
            .id("route-header")
            .h(px(HEADER_H))
            .px(px(10.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .flex_shrink_0()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .text_color(theme.fg_default)
                    .child(route.title.clone()),
            )
            .child(div().flex_shrink_0().font(mono_font()).text_size(px(11.)).text_color(theme.fg_dim).child(format!("{done}/{total}")))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_route_map(cx)));

        let n = route.stations.len();
        let rows: Vec<AnyElement> = route
            .stations
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let (mark, color): (AnyElement, Rgba) = match s.state {
                    StationState::Done => (icon(IconName::Check, theme.fg_dim).size(px(12.)).into_any_element(), theme.fg_dim),
                    // 実行中の印は、動く輪として上に重ねる。
                    StationState::Current => (div().into_any_element(), theme.accent),
                    StationState::Todo => (
                        div().size(px(8.)).rounded_full().border_1().border_color(theme.fg_dim).bg(theme.bg_window).into_any_element(),
                        theme.fg_default,
                    ),
                };
                let open = s.line.zip(route.plan_rel.clone()).map(|(line, rel)| (line, plan_location(&root, &rel)));
                let row = div()
                    .id(ElementId::Name(format!("route-station-{i}").into()))
                    .h(px(STATION_H))
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(div().w(px(MARK_W)).flex_shrink_0().flex().items_center().justify_center().child(mark))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pr(px(8.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(color)
                            .child(s.label.clone()),
                    );
                match open {
                    Some((line, file)) => row
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_file(file.clone(), None, Some((line, None)), window, cx);
                        }))
                        .into_any_element(),
                    None => row.into_any_element(),
                }
            })
            .collect();
        let line_x = MARK_W / 2.0;
        let ring = route.current().map(|_| {
            div()
                .absolute()
                .left(px((MARK_W - RING) / 2.0))
                .top(px(ring_y(self.ring.value(now))))
                .size(px(RING))
                .rounded_full()
                .border_2()
                .border_color(theme.accent)
                .bg(theme.bg_window)
        });
        let stations = div()
            .relative()
            .h(px(n as f32 * STATION_H))
            .flex_shrink_0()
            // 駅をつなぐ縦の線。
            .child(
                div()
                    .absolute()
                    .left(px(line_x))
                    .top(px(STATION_H / 2.0))
                    .w(px(1.))
                    .h(px((n.saturating_sub(1)) as f32 * STATION_H))
                    .bg(theme.border_subtle),
            )
            .children(rows)
            .children(ring);
        div()
            .w(px(ROUTE_W))
            .h_full()
            .flex()
            .flex_col()
            .child(header)
            .child(div().id("route-stations").flex_1().min_h_0().overflow_y_scroll().child(stations))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{STATION_H, ledger_removed, plan_location, ring_y, route_width};
    use fude_fs::RelPath;
    use fude_fs::plans::PlanStamp;

    #[test]
    fn finished_plan_is_hidden_when_its_ledger_goes() {
        let stamp = |rel: &str, plan: u64, ledger: Option<u64>| PlanStamp {
            rel: RelPath::new(rel).unwrap(),
            plan_mtime_ms: plan,
            ledger_mtime_ms: ledger,
        };
        let running = stamp("docs/superpowers/plans/p.md", 10, Some(20));
        assert!(ledger_removed(Some(&running), &stamp("docs/superpowers/plans/p.md", 10, None)));
        // プランが書き換わった、別のプランになった、初めて見た、台帳が増えた、はどれも違う。
        assert!(!ledger_removed(Some(&running), &stamp("docs/superpowers/plans/p.md", 11, None)));
        assert!(!ledger_removed(Some(&running), &stamp("docs/superpowers/plans/q.md", 10, None)));
        assert!(!ledger_removed(None, &stamp("docs/superpowers/plans/p.md", 10, None)));
        assert!(!ledger_removed(Some(&stamp("docs/superpowers/plans/p.md", 10, None)), &running));
    }
    use fude_core::location::FileLocation;
    use fude_core::workspace::ProjectRoot;
    use std::path::PathBuf;

    #[test]
    fn widths() {
        assert_eq!(route_width(false, false), 0.0);
        assert_eq!(route_width(false, true), 0.0);
        assert_eq!(route_width(true, false), 180.0);
        assert_eq!(route_width(true, true), 28.0);
    }

    #[test]
    fn ring_moves_by_station() {
        assert_eq!(ring_y(0.0), ring_y(1.0) - STATION_H);
        assert_eq!(ring_y(1.5), ring_y(1.0) + STATION_H / 2.0);
    }

    #[test]
    fn plan_location_joins_root() {
        let local = plan_location(&ProjectRoot::Local(PathBuf::from(r"C:\w\p")), "docs/superpowers/plans/a.md");
        assert_eq!(
            local,
            FileLocation::Local(PathBuf::from(r"C:\w\p").join("docs").join("superpowers").join("plans").join("a.md"))
        );
        let wsl = plan_location(&ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/x/p".into() }, "docs/a.md");
        assert_eq!(wsl, FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/x/p/docs/a.md".into() });
    }
}
