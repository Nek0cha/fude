//! タブの出入り、分割の境目、ターミナルのフェードインの動き（設計書 §6.3）。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use fude_core::motion::Fade;
use fude_core::workspace::{Project, ProjectId, TabId};

use crate::theme::motion::{REDUCED, SPLIT, TAB_IN, TAB_OUT, TAB_TEXT_OUT, ease_out, ease_out_quint};
use crate::theme::size::TAB_MAX_W;

/// ペインの数に応じた、左のペインの幅の割合。
pub(crate) fn split_target(panes: usize) -> f32 {
    if panes >= 2 { 0.5 } else { 1.0 }
}

fn frac(elapsed: Duration, total: Duration) -> f32 {
    if total.is_zero() {
        return 1.0;
    }
    (elapsed.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
}

/// 出てきたタブの幅と文字の濃さ（0〜1）。動きを減らす設定では 1。
pub(crate) fn tab_open_progress(elapsed: Duration, reduced: bool) -> f32 {
    if reduced {
        return 1.0;
    }
    ease_out_quint(frac(elapsed, TAB_IN))
}

/// 閉じたタブの (文字の濃さ, 幅の割合, 消え終わったか)。文字を 80ms で消してから、幅を 140ms で縮める。
/// 動きを減らす設定では、幅は 0 にして、文字だけを 80ms で消す。
pub(crate) fn tab_close_progress(elapsed: Duration, reduced: bool) -> (f32, f32, bool) {
    if reduced {
        let t = frac(elapsed, REDUCED);
        return (1.0 - ease_out(t), 0.0, t >= 1.0);
    }
    let text = 1.0 - ease_out(frac(elapsed, TAB_TEXT_OUT));
    let w = frac(elapsed.saturating_sub(TAB_TEXT_OUT), TAB_OUT);
    (text, 1.0 - ease_out_quint(w), w >= 1.0)
}

/// タブの幅の見積もり（px）。余白、アイコン、×、文字の幅から。TAB_MAX_W を超えない。
pub(crate) fn tab_width_estimate(title: &str) -> f32 {
    let chars: f32 = title.chars().map(|c| if c.is_ascii() { 7.5 } else { 13.0 }).sum();
    (62.0 + chars).min(f32::from(TAB_MAX_W))
}

/// タブの形（id、題名、ペインの数）。
type TabShape = (TabId, String, usize);

/// 閉じて、縮みながら消えていくタブ。
pub(crate) struct ClosingTab {
    pub index: usize,
    pub title: String,
    pub started: Instant,
}

/// タブと分割の動きの状態。描画のたびに observe で今の形と比べ、変わったところの動きを始める。
#[derive(Default)]
pub(crate) struct LayoutAnims {
    pub opened: HashMap<TabId, Instant>,
    pub closing: Vec<ClosingTab>,
    /// タブごとの左のペインの幅の割合。
    pub split: HashMap<TabId, Fade>,
    prev: Option<(ProjectId, Vec<TabShape>)>,
}

impl LayoutAnims {
    pub fn observe(&mut self, project: Option<&Project>, now: Instant, reduced: bool) {
        let current: Option<(ProjectId, Vec<TabShape>)> =
            project.map(|p| (p.id, p.tabs.iter().map(|t| (t.id, t.title(), t.panes.len())).collect()));
        let same_project = matches!((&self.prev, &current), (Some((a, _)), Some((b, _))) if a == b);
        if same_project {
            let (_, before) = self.prev.as_ref().expect("same_project");
            let (_, after) = current.as_ref().expect("same_project");
            for (i, (id, title, _)) in before.iter().enumerate() {
                if !after.iter().any(|(a, _, _)| a == id) {
                    self.closing.push(ClosingTab { index: i, title: title.clone(), started: now });
                }
            }
            for (id, _, _) in after {
                if !before.iter().any(|(b, _, _)| b == id) {
                    self.opened.insert(*id, now);
                }
            }
        } else {
            self.opened.clear();
            self.closing.clear();
        }
        // 分割の境目。ペインの数が変わったタブだけ動かす。
        if let Some((_, tabs)) = &current {
            for (id, _, panes) in tabs {
                let target = split_target(*panes);
                let fade = self.split.entry(*id).or_insert_with(|| Fade::settled(target));
                let duration = if reduced { Duration::ZERO } else { SPLIT };
                fade.animate_to(target, duration, ease_out_quint, now);
            }
            self.split.retain(|id, _| tabs.iter().any(|(t, _, _)| t == id));
        }
        self.opened.retain(|_, t| now.saturating_duration_since(*t) < TAB_IN);
        self.closing.retain(|c| !tab_close_progress(now.saturating_duration_since(c.started), reduced).2);
        self.prev = current;
    }

    pub fn split_share(&self, tab: TabId, now: Instant) -> f32 {
        self.split.get(&tab).map_or(1.0, |f| f.value(now))
    }

    /// タブの出入りか分割の境目が動いている。
    pub fn tabs_animating(&self, now: Instant) -> bool {
        !self.opened.is_empty() || !self.closing.is_empty() || self.split_animating(now)
    }

    pub fn split_animating(&self, now: Instant) -> bool {
        self.split.values().any(|f| f.is_animating(now))
    }
}

#[cfg(test)]
mod tests {
    use super::{split_target, tab_close_progress, tab_open_progress, tab_width_estimate};
    use crate::theme::motion::{TAB_IN, TAB_OUT, TAB_TEXT_OUT};
    use std::time::Duration;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn split_share_targets() {
        assert_eq!(split_target(1), 1.0);
        assert_eq!(split_target(2), 0.5);
    }

    #[test]
    fn tab_open_grows_over_160ms() {
        assert_eq!(tab_open_progress(ms(0), false), 0.0);
        assert!(tab_open_progress(ms(80), false) > 0.5);
        assert_eq!(tab_open_progress(TAB_IN, false), 1.0);
        // 動きを減らす設定では、幅はすぐに戻す。
        assert_eq!(tab_open_progress(ms(0), true), 1.0);
    }

    #[test]
    fn closing_tab_removed_after_out() {
        // (文字の濃さ, 幅の割合, 消え終わったか)
        assert_eq!(tab_close_progress(ms(0), false), (1.0, 1.0, false));
        let (text, width, gone) = tab_close_progress(TAB_TEXT_OUT, false);
        assert_eq!((text, width, gone), (0.0, 1.0, false));
        let (_, width, _) = tab_close_progress(TAB_TEXT_OUT + ms(70), false);
        assert!(width < 1.0 && width > 0.0);
        assert_eq!(tab_close_progress(TAB_TEXT_OUT + TAB_OUT, false), (0.0, 0.0, true));
        // 動きを減らす設定では、幅は 0 のまま、文字だけ 80ms で消す。
        assert!(tab_close_progress(ms(80), true).2);
        assert_eq!(tab_close_progress(ms(0), true).1, 0.0);
    }

    #[test]
    fn tab_width_estimate_caps() {
        assert!(tab_width_estimate("pwsh") < 120.0);
        assert_eq!(tab_width_estimate(&"x".repeat(100)), 200.0);
    }
}
