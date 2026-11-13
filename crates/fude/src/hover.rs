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

#[derive(Default)]
pub struct HoverFades {
    fades: HashMap<HoverKey, Fade>,
}

impl HoverFades {
    pub fn set(&mut self, key: HoverKey, hovered: bool, now: Instant) {
        let (target, duration) = if hovered { (1.0, HOVER_IN) } else { (0.0, HOVER_OUT) };
        self.fades.entry(key).or_insert_with(|| Fade::settled(0.0)).animate_to(target, duration, ease_out, now);
    }

    /// すべての部品のホバーを外す。カーソルがウィンドウの外へ出たときに使う。
    pub fn release_all(&mut self, now: Instant) {
        for fade in self.fades.values_mut() {
            fade.animate_to(0.0, HOVER_OUT, ease_out, now);
        }
    }

    /// 0（ホバーしていない）〜 1（ホバー中）。登録の無い部品は 0。
    pub fn value(&self, key: HoverKey, now: Instant) -> f32 {
        self.fades.get(&key).map_or(0.0, |f| f.value(now))
    }

    pub fn any_animating(&self, now: Instant) -> bool {
        self.fades.values().any(|f| f.is_animating(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// カーソルがウィンドウの外へ出たとき、部品ごとの「外れた」通知は届かないため、まとめて消す。
    #[test]
    fn release_all_fades_out_every_key() {
        let t0 = Instant::now();
        let mut fades = HoverFades::default();
        fades.set(HoverKey::CaptionClose, true, t0);
        fades.set(HoverKey::AddTab, true, t0);
        fades.release_all(t0 + ms(80));
        assert_eq!(fades.value(HoverKey::CaptionClose, t0 + ms(200)), 0.0);
        assert_eq!(fades.value(HoverKey::AddTab, t0 + ms(200)), 0.0);
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
