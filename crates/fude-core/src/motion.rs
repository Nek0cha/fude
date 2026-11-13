//! 時間で値を補間する。ホバーのフェードやパネルの出入りに使う。

use std::time::{Duration, Instant};

/// 進み具合（0〜1）を、補間に使う割合へ変換する関数。
pub type Easing = fn(f32) -> f32;

/// from から to へ、duration をかけて値を動かす。
/// 動きの途中で目標を変えると、その時点の値から動き直す。
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
    easing: Easing,
}

impl Fade {
    /// 動いていない状態で value に止まっている Fade。
    pub fn settled(value: f32) -> Self {
        Self { from: value, to: value, start: Instant::now(), duration: Duration::ZERO, easing: |t| t }
    }

    pub fn value(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return self.to;
        }
        let elapsed = now.saturating_duration_since(self.start);
        let p = (elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        if p >= 1.0 {
            return self.to;
        }
        self.from + (self.to - self.from) * (self.easing)(p)
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    /// 今の値から target へ動かし始める。target が今の目標と同じなら何もしない。
    pub fn animate_to(&mut self, target: f32, duration: Duration, easing: Easing, now: Instant) {
        if target == self.to {
            return;
        }
        self.from = self.value(now);
        self.to = target;
        self.start = now;
        self.duration = duration;
        self.easing = easing;
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) < self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const LINEAR: Easing = |t| t;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn settled_value_is_constant() {
        let t0 = Instant::now();
        let f = Fade::settled(0.0);
        assert_eq!(f.value(t0 + ms(500)), 0.0);
        assert!(!f.is_animating(t0 + ms(500)));
    }

    #[test]
    fn linear_midpoint() {
        let t0 = Instant::now();
        let mut f = Fade::settled(0.0);
        f.animate_to(1.0, ms(100), LINEAR, t0);
        assert!(close(f.value(t0 + ms(50)), 0.5));
        assert!(f.is_animating(t0 + ms(50)));
    }

    #[test]
    fn clamps_after_duration() {
        let t0 = Instant::now();
        let mut f = Fade::settled(0.0);
        f.animate_to(1.0, ms(100), LINEAR, t0);
        assert_eq!(f.value(t0 + ms(250)), 1.0);
        assert!(!f.is_animating(t0 + ms(250)));
    }

    #[test]
    fn retarget_midway_starts_from_current_value() {
        let t0 = Instant::now();
        let mut f = Fade::settled(0.0);
        f.animate_to(1.0, ms(100), LINEAR, t0);
        f.animate_to(0.0, ms(120), LINEAR, t0 + ms(40));
        assert!(close(f.value(t0 + ms(40)), 0.4));
        assert!(close(f.value(t0 + ms(100)), 0.2));
        assert_eq!(f.target(), 0.0);
    }

    #[test]
    fn same_target_does_not_restart() {
        let t0 = Instant::now();
        let mut f = Fade::settled(0.0);
        f.animate_to(1.0, ms(100), LINEAR, t0);
        f.animate_to(1.0, ms(100), LINEAR, t0 + ms(30));
        assert!(close(f.value(t0 + ms(50)), 0.5));
    }

    #[test]
    fn zero_duration_jumps() {
        let t0 = Instant::now();
        let mut f = Fade::settled(0.0);
        f.animate_to(1.0, Duration::ZERO, LINEAR, t0);
        assert_eq!(f.value(t0), 1.0);
    }

    #[test]
    fn easing_is_applied() {
        let t0 = Instant::now();
        let mut f = Fade::settled(0.0);
        f.animate_to(1.0, ms(100), |t| t * t, t0);
        assert!(close(f.value(t0 + ms(50)), 0.25));
    }
}
