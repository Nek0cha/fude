//! 出入りする部品の動き（設計書 §6.3）。消える動きの間も描き続けるため、状態を持つ。

use std::time::{Duration, Instant};

use fude_core::motion::Fade;

use crate::theme::motion::{REDUCED, ease_out, ease_out_quint};

/// 動きの種類。OS の「動きを減らす」がオンなら、移動や拡大をやめて不透明度だけにする。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    Move,
    FadeOnly,
}

/// 動きの長さと種類。reduced なら (REDUCED, FadeOnly)。
pub(crate) fn plan(duration: Duration, reduced: bool) -> (Duration, Motion) {
    if reduced { (REDUCED, Motion::FadeOnly) } else { (duration, Motion::Move) }
}

/// 出入りする部品の状態。value は 0（消えている）〜 1（出ている）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Presence {
    fade: Fade,
    motion: Motion,
}

impl Presence {
    pub fn hidden() -> Presence {
        Presence { fade: Fade::settled(0.0), motion: Motion::Move }
    }

    pub fn shown() -> Presence {
        Presence { fade: Fade::settled(1.0), motion: Motion::Move }
    }

    pub fn show(&mut self, duration: Duration, reduced: bool, now: Instant) {
        let (d, m) = plan(duration, reduced);
        self.motion = m;
        let easing = if m == Motion::FadeOnly { ease_out } else { ease_out_quint };
        self.fade.animate_to(1.0, d, easing, now);
    }

    pub fn hide(&mut self, duration: Duration, reduced: bool, now: Instant) {
        let (d, m) = plan(duration, reduced);
        self.motion = m;
        let easing = if m == Motion::FadeOnly { ease_out } else { ease_out_quint };
        self.fade.animate_to(0.0, d, easing, now);
    }

    pub fn value(&self, now: Instant) -> f32 {
        self.fade.value(now)
    }

    /// 移動に使う割合（FadeOnly なら常に 1 = 動かさない）。
    pub fn travel(&self, now: Instant) -> f32 {
        if self.motion == Motion::FadeOnly { 1.0 } else { self.value(now) }
    }

    /// 目標が「出ている」か。
    pub fn visible(&self) -> bool {
        self.fade.target() > 0.5
    }

    /// 消え終わった（描かなくてよい）。
    pub fn is_gone(&self, now: Instant) -> bool {
        !self.visible() && !self.fade.is_animating(now)
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.fade.is_animating(now)
    }
}

#[cfg(test)]
mod tests {
    use super::{Motion, Presence, plan};
    use crate::theme::motion::{PANEL_IN, PANEL_OUT, REDUCED};
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn plan_uses_reduced_duration() {
        assert_eq!(plan(PANEL_IN, false), (PANEL_IN, Motion::Move));
        assert_eq!(plan(PANEL_IN, true), (REDUCED, Motion::FadeOnly));
    }

    #[test]
    fn presence_keeps_drawing_until_gone() {
        let t0 = Instant::now();
        let mut p = Presence::hidden();
        assert!(p.is_gone(t0));
        p.show(PANEL_IN, false, t0);
        assert!(!p.is_gone(t0));
        assert_eq!(p.value(t0 + PANEL_IN), 1.0);
        p.hide(PANEL_OUT, false, t0 + ms(500));
        assert!(!p.is_gone(t0 + ms(510)), "消える動きの間は描き続ける");
        assert!(p.is_gone(t0 + ms(500) + PANEL_OUT));
        // 途中で出し直すと、今の値から戻る（最初からやり直さない）。
        let mut q = Presence::hidden();
        q.show(PANEL_IN, false, t0);
        q.hide(PANEL_OUT, false, t0 + ms(60));
        let mid = q.value(t0 + ms(100));
        q.show(PANEL_IN, false, t0 + ms(100));
        assert!((q.value(t0 + ms(100)) - mid).abs() < 1e-6);
        assert!(q.visible());
    }
}
