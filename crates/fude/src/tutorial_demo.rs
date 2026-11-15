//! チュートリアルのツリーの動く絵（9c の設計書 §5.3）。画面の模型を今のテーマの色で描く。

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::Theme;
use crate::theme::motion::ease_out;

/// 1 周の長さ。
pub(crate) const DEMO_PERIOD: Duration = Duration::from_millis(3000);
/// カーソルの休む位置と、ピルの中央（模型の左上を (0,0)、右下を (1,1) とする）。
/// ピルは実際の画面と同じく、上の列の右端（Windows では窓の操作ボタンの左）に置く。
pub(crate) const REST: (f32, f32) = (0.45, 0.8);
pub(crate) const PILL: (f32, f32) = ((DEMO_W - TITLE_TAIL_W - 4.0 - PILL_W / 2.0) / DEMO_W, TITLE_H / 2.0 / DEMO_H);

/// 模型の大きさ（動く絵）と、静止画の 1 枚の大きさ。
const DEMO_W: f32 = 280.0;
const DEMO_H: f32 = 150.0;
const STILL_W: f32 = 190.0;
const STILL_H: f32 = 102.0;
/// 模型の上の列の高さと、ピルの幅（動く絵の大きさでの値）。
const TITLE_H: f32 = 20.0;
const PILL_W: f32 = 68.0;
/// 上の列の右端で、ピルより右に取る幅（Windows は窓の操作ボタン 3 つ、ほかは余白だけ）。
const CAPTION_W: f32 = 12.0;
const TITLE_TAIL_W: f32 = if cfg!(windows) { CAPTION_W * 3.0 + 6.0 } else { 8.0 };

/// ある時刻の模型の形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DemoFrame {
    pub cursor: (f32, f32),
    pub pressed: bool,
    /// 0.0（閉じた）〜1.0（開いた）。
    pub tree_open: f32,
}

fn lerp(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// 時刻 t の形。0〜800ms でピルへ動き、800〜950ms で押し、950〜1250ms でツリーが開き、1250〜2450ms は止まり、
/// 2450〜3000ms でツリーが閉じてカーソルが戻る。
pub(crate) fn demo_frame(t: Duration) -> DemoFrame {
    let ms = (t.as_millis() % DEMO_PERIOD.as_millis()) as f32;
    let phase = |from: f32, to: f32| ease_out(((ms - from) / (to - from)).clamp(0.0, 1.0));
    match ms {
        m if m < 800.0 => DemoFrame { cursor: lerp(REST, PILL, phase(0.0, 800.0)), pressed: false, tree_open: 0.0 },
        m if m < 950.0 => DemoFrame { cursor: PILL, pressed: true, tree_open: 0.0 },
        m if m < 1250.0 => DemoFrame { cursor: PILL, pressed: false, tree_open: phase(950.0, 1250.0) },
        m if m < 2450.0 => DemoFrame { cursor: PILL, pressed: false, tree_open: 1.0 },
        _ => {
            let p = phase(2450.0, 3000.0);
            DemoFrame { cursor: lerp(PILL, REST, p), pressed: false, tree_open: 1.0 - p }
        }
    }
}

/// 模型 1 枚。w と h は外枠の大きさ。並びは実際の画面と同じく、左からレール、ターミナル、ツリー（右端から出る）。
fn mock(frame: DemoFrame, w: f32, h: f32, theme: &Theme) -> Div {
    let k = w / DEMO_W;
    let title_h = TITLE_H * k;
    let rail_w = 14.0 * k;
    let tree_w = 80.0 * k * frame.tree_open;
    let text = px((8.0 * k).max(6.0));
    let line = |width: f32| div().h(px(3.0 * k)).w(px(width * k)).rounded(px(1.5 * k)).bg(theme.border_subtle);
    // Windows の窓の操作ボタン（最小化、最大化、閉じる）の印。
    let captions = div()
        .absolute()
        .top(px(0.))
        .right(px(3.0 * k))
        .h(px(title_h))
        .flex()
        .flex_row()
        .items_center()
        .children((0..3).map(|_| {
            div().w(px(CAPTION_W * k)).flex().justify_center().child(div().w(px(5.0 * k)).h(px(1.0 * k).max(px(1.))).bg(theme.fg_dim))
        }));
    let pill = div()
        .absolute()
        .left(px(PILL.0 * w - PILL_W / 2.0 * k))
        .top(px(PILL.1 * h - 6.0 * k))
        .w(px(PILL_W * k))
        .h(px(12.0 * k))
        .rounded(px(6.0 * k))
        .border_1()
        .border_color(if frame.pressed { theme.accent } else { theme.border_subtle })
        .bg(theme.bg_hover)
        .flex()
        .items_center()
        .justify_center()
        .text_size(text)
        .text_color(if frame.pressed { theme.accent } else { theme.fg_default })
        .child("app · main");
    let terminal = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .p(px(8.0 * k))
        .flex()
        .flex_col()
        .gap(px(5.0 * k))
        .children([line(90.0), line(60.0), line(110.0), line(40.0), line(75.0)]);
    let tree = div()
        .w(px(tree_w))
        .h_full()
        .flex_shrink_0()
        .overflow_hidden()
        .when(frame.tree_open > 0.0, |d| d.border_l_1().border_color(theme.border_subtle))
        .bg(theme.bg_panel)
        .opacity(frame.tree_open)
        .p(px(6.0 * k))
        .flex()
        .flex_col()
        .gap(px(3.0 * k))
        .text_size(text)
        .text_color(theme.fg_dim)
        .children(["src/", "  main.rs", "  lib.rs", "Cargo.toml"].map(|n| div().whitespace_nowrap().child(n)));
    let cursor = div()
        .absolute()
        .left(px(frame.cursor.0 * w - 5.0 * k))
        .top(px(frame.cursor.1 * h - 5.0 * k))
        .size(px(10.0 * k))
        .rounded(px(5.0 * k))
        .bg(theme.fg_default)
        .border_2()
        .border_color(if frame.pressed { theme.accent } else { theme.bg_window });
    div()
        .relative()
        .w(px(w))
        .h(px(h))
        .flex_shrink_0()
        .overflow_hidden()
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border_subtle)
        .bg(theme.bg_window)
        .flex()
        .flex_col()
        .child(div().h(px(title_h)).w_full().flex_shrink_0().border_b_1().border_color(theme.border_subtle))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_row()
                .child(div().w(px(rail_w)).h_full().flex_shrink_0().border_r_1().border_color(theme.border_subtle))
                .child(terminal)
                .child(tree),
        )
        .when(cfg!(windows), |d| d.child(captions))
        .child(pill)
        .child(cursor)
}

/// ツリーの開き方の絵と、添える文。frame が None なら動きを減らす設定の静止画（閉じた模型 → 開いた模型）。
/// keys はツリーを開くキーの表示（空なら書かない）。
pub(crate) fn tree_demo(frame: Option<DemoFrame>, keys: &str, theme: &Theme) -> Div {
    let picture = match frame {
        Some(f) => div().child(mock(f, DEMO_W, DEMO_H, theme)),
        None => {
            let closed = DemoFrame { cursor: PILL, pressed: true, tree_open: 0.0 };
            let open = DemoFrame { cursor: PILL, pressed: false, tree_open: 1.0 };
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(mock(closed, STILL_W, STILL_H, theme))
                .child(div().text_color(theme.fg_dim).child("→"))
                .child(mock(open, STILL_W, STILL_H, theme))
        }
    };
    let caption = if keys.is_empty() { "ここをクリック".to_string() } else { format!("ここをクリック（{keys} でも開きます）") };
    div().flex().flex_col().gap(px(6.)).child(picture).child(div().text_size(px(12.)).text_color(theme.fg_dim).child(caption))
}

#[cfg(test)]
mod tests {
    use super::{DEMO_H, DEMO_PERIOD, DEMO_W, PILL, PILL_W, REST, TITLE_H, TITLE_TAIL_W, demo_frame};
    use std::time::Duration;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn demo_starts_at_rest_closed() {
        let f = demo_frame(ms(0));
        assert_eq!(f.cursor, REST);
        assert_eq!(f.tree_open, 0.0);
        assert!(!f.pressed);
    }

    #[test]
    fn demo_presses_pill() {
        let f = demo_frame(ms(850));
        assert_eq!(f.cursor, PILL);
        assert!(f.pressed);
    }

    #[test]
    fn demo_tree_open_while_holding() {
        assert_eq!(demo_frame(ms(1800)).tree_open, 1.0);
        assert!(!demo_frame(ms(1800)).pressed);
    }

    #[test]
    fn pill_sits_at_right_of_title_row() {
        // 実際の画面と同じく、ピルは上の列の右端（Windows では窓の操作ボタンの左）にある。
        let (x, y) = PILL;
        let half_w = PILL_W / 2.0 / DEMO_W;
        let right = 1.0 - (TITLE_TAIL_W + 4.0) / DEMO_W;
        assert!(x > 0.6, "右に寄せる: {x}");
        assert!(x + half_w <= right + 0.001, "窓の操作ボタンに重ねない: {x}");
        assert!(y < TITLE_H / DEMO_H, "上の列の中: {y}");
    }

    #[test]
    fn demo_loops() {
        assert_eq!(DEMO_PERIOD, ms(3000));
        assert_eq!(demo_frame(ms(3000)), demo_frame(ms(0)));
        assert_eq!(demo_frame(ms(4800)), demo_frame(ms(1800)));
    }
}
