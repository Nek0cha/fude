//! ホバーの背景のフェード。GPUI の .hover() は瞬時に切り替わるため、状態を自前で持つ。

use std::collections::HashMap;
use std::time::Instant;

use fude_core::motion::Fade;
use fude_core::workspace::{ProjectId, TabId};

use crate::theme::motion::{HOVER_IN, HOVER_OUT, ease_out};

/// ホバーでフェードする部品。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HoverKey {
    Project(ProjectId),
    AddProject,
    Tab(TabId),
    TabClose(TabId),
    AddTab,
    Split,
    CaptionMin,
    CaptionMax,
    CaptionClose,
}

/// 部品ごとのホバーの状態と、カーソルがウィンドウの上にあるかの状態を持つ。
///
/// カーソルがウィンドウの外へ出ても、GPUI は最後の位置で判定を続けるため、部品の
/// on_hover(false) は呼ばれない。部品ごとの状態は GPUI の判定と揃えたまま残し、
/// ウィンドウ全体の濃さ（window）を掛けて表示から消す。戻ると、そのまま色が戻る。
pub struct HoverFades {
    fades: HashMap<HoverKey, Fade>,
    window: Fade,
}

impl Default for HoverFades {
    fn default() -> Self {
        Self { fades: HashMap::new(), window: Fade::settled(1.0) }
    }
}

impl HoverFades {
    pub fn set(&mut self, key: HoverKey, hovered: bool, now: Instant) {
        let (target, duration) = if hovered { (1.0, HOVER_IN) } else { (0.0, HOVER_OUT) };
        self.fades.entry(key).or_insert_with(|| Fade::settled(0.0)).animate_to(target, duration, ease_out, now);
    }

    /// カーソルがウィンドウの上にあるかを伝える。描画のたびに呼んでよい。
    pub fn set_window_hovered(&mut self, hovered: bool, now: Instant) {
        let (target, duration) = if hovered { (1.0, HOVER_IN) } else { (0.0, HOVER_OUT) };
        self.window.animate_to(target, duration, ease_out, now);
    }

    /// 0（ホバーしていない）〜 1（ホバー中）。登録の無い部品は 0。
    pub fn value(&self, key: HoverKey, now: Instant) -> f32 {
        self.fades.get(&key).map_or(0.0, |f| f.value(now)) * self.window.value(now)
    }

    pub fn any_animating(&self, now: Instant) -> bool {
        self.window.is_animating(now) || self.fades.values().any(|f| f.is_animating(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// ボタンの上から外へ出て、同じボタンの上へ戻る。GPUI はボタンを「ホバー中」のまま扱い、
    /// 通知を送り直さないため、戻った時点で色が出なければならない。
    #[test]
    fn return_to_same_button_restores_color() {
        let t0 = Instant::now();
        let mut fades = HoverFades::default();
        fades.set_window_hovered(true, t0);
        fades.set(HoverKey::CaptionMin, true, t0);
        fades.set_window_hovered(false, t0 + ms(100));
        assert_eq!(fades.value(HoverKey::CaptionMin, t0 + ms(300)), 0.0);
        fades.set_window_hovered(true, t0 + ms(400));
        assert_eq!(fades.value(HoverKey::CaptionMin, t0 + ms(600)), 1.0);
    }

    /// 「外」の判定が一瞬はさまっても、そのあいだに届いたホバーは戻ったあと効く。
    #[test]
    fn hover_set_while_outside_shows_after_return() {
        let t0 = Instant::now();
        let mut fades = HoverFades::default();
        fades.set_window_hovered(true, t0);
        fades.set_window_hovered(false, t0 + ms(10));
        fades.set(HoverKey::CaptionMax, true, t0 + ms(10));
        fades.set_window_hovered(true, t0 + ms(12));
        assert_eq!(fades.value(HoverKey::CaptionMax, t0 + ms(200)), 1.0);
    }

    #[test]
    fn window_leave_after_hover_fades_out() {
        let t0 = Instant::now();
        let mut fades = HoverFades::default();
        fades.set_window_hovered(true, t0);
        fades.set(HoverKey::CaptionClose, true, t0);
        fades.set_window_hovered(false, t0 + ms(80));
        assert_eq!(fades.value(HoverKey::CaptionClose, t0 + ms(300)), 0.0);
    }

    #[test]
    fn unknown_key_is_zero() {
        let fades = HoverFades::default();
        assert_eq!(fades.value(HoverKey::AddTab, Instant::now()), 0.0);
    }

    #[test]
    fn hover_in_reaches_one() {
        let t0 = Instant::now();
        let mut fades = HoverFades::default();
        fades.set(HoverKey::AddTab, true, t0);
        assert_eq!(fades.value(HoverKey::AddTab, t0 + ms(80)), 1.0);
    }

    #[test]
    fn hover_out_takes_120ms() {
        let t0 = Instant::now();
        let mut fades = HoverFades::default();
        fades.set(HoverKey::AddTab, true, t0);
        fades.set(HoverKey::AddTab, false, t0 + ms(80));
        assert!(fades.value(HoverKey::AddTab, t0 + ms(80 + 119)) > 0.0);
        assert_eq!(fades.value(HoverKey::AddTab, t0 + ms(200)), 0.0);
        assert!(!fades.any_animating(t0 + ms(200)));
    }
}
