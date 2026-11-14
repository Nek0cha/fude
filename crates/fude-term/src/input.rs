//! 入力に関わる小さな変換。貼り付け、ホイール、代替スクロール。

/// 貼り付ける文字列をバイト列にする。CRLF と LF を CR にそろえる。
/// bracketed なら中の ESC を取り除き、`ESC[200~` と `ESC[201~` で包む（Claude Code などに 1 行ずつ実行させない）。
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if !bracketed {
        return normalized.into_bytes();
    }
    let mut out = b"\x1b[200~".to_vec();
    out.extend(normalized.bytes().filter(|&b| b != 0x1b));
    out.extend_from_slice(b"\x1b[201~");
    out
}

/// ホイールの量（行。小数あり）を積算し、整数の行だけを返す。
#[derive(Default)]
pub struct WheelAccum {
    remainder: f32,
}

impl WheelAccum {
    /// 向きが変わったら、それまでの端数を捨てる。
    pub fn feed(&mut self, lines: f32) -> i32 {
        if lines * self.remainder < 0.0 {
            self.remainder = 0.0;
        }
        self.remainder += lines;
        let whole = self.remainder.trunc();
        self.remainder -= whole;
        whole as i32
    }
}

/// Windows のホイール 1 段で届く行数（OS の既定値）。
const WINDOWS_LINES_PER_NOTCH: f32 = 3.0;

/// 代替スクロールで送る行数。Windows ではホイール 1 段を 1 行にする（ユーザーの決定）。
pub fn alt_scroll_lines(lines: f32, windows: bool) -> f32 {
    if windows { lines / WINDOWS_LINES_PER_NOTCH } else { lines }
}

/// 代替スクロール。正（上）は上矢印、負は下矢印を |lines| 回送る。app_cursor なら `ESC O` 形式。
pub fn alt_scroll_bytes(lines: i32, app_cursor: bool) -> Vec<u8> {
    let arrow = if lines > 0 { b'A' } else { b'B' };
    let one = if app_cursor { [0x1b, b'O', arrow] } else { [0x1b, b'[', arrow] };
    one.repeat(lines.unsigned_abs() as usize)
}

/// アプリが求めたマウスの報告の種類（DECSET 1000 / 1002 / 1003）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseReport {
    Off,
    /// 押したときと離したときだけ（1000）。
    Click,
    /// それに加えて、押したままの移動（1002）。
    Drag,
    /// すべての移動（1003）。
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseMode {
    pub report: MouseReport,
    /// SGR の形式（1006）で送るか。
    pub sgr: bool,
}

impl MouseMode {
    pub fn is_on(self) -> bool {
        self.report != MouseReport::Off
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKey {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// ボタンを押していない移動。
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Move,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseMods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// マウスの報告のバイト列。row と col は 0 始まり。モードが求めていない出来事は None。
pub fn mouse_bytes(mode: MouseMode, key: MouseKey, action: MouseAction, row: u16, col: u16, mods: MouseMods) -> Option<Vec<u8>> {
    let wanted = match (mode.report, action, key) {
        (MouseReport::Off, _, _) => false,
        (_, MouseAction::Press | MouseAction::Release, _) => true,
        (MouseReport::Motion, MouseAction::Move, _) => true,
        (MouseReport::Drag, MouseAction::Move, k) => k != MouseKey::None,
        (MouseReport::Click, MouseAction::Move, _) => false,
    };
    if !wanted {
        return None;
    }
    let base: u32 = match key {
        MouseKey::Left => 0,
        MouseKey::Middle => 1,
        MouseKey::Right => 2,
        MouseKey::None => 3,
        MouseKey::WheelUp => 64,
        MouseKey::WheelDown => 65,
    };
    let mut code = base;
    if action == MouseAction::Move {
        code += 32;
    }
    code += u32::from(mods.shift) * 4 + u32::from(mods.alt) * 8 + u32::from(mods.ctrl) * 16;
    let (x, y) = (u32::from(col) + 1, u32::from(row) + 1);
    if mode.sgr {
        let end = if action == MouseAction::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    // 古い形式は、離したときのボタンを 3 にし、値に 32 を足して 1 バイトで送る。
    if action == MouseAction::Release {
        code = code - base + 3;
    }
    let byte = |v: u32| u8::try_from(v + 32).ok();
    Some(vec![0x1b, b'[', b'M', byte(code)?, byte(x)?, byte(y)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_plain_normalizes_newlines() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc");
    }

    #[test]
    fn paste_bracketed_strips_esc_and_wraps() {
        assert_eq!(paste_bytes("x\x1b[201~y\n", true), b"\x1b[200~x[201~y\r\x1b[201~");
    }

    #[test]
    fn wheel_accumulates_fractions() {
        let mut w = WheelAccum::default();
        assert_eq!(w.feed(0.4), 0);
        assert_eq!(w.feed(0.7), 1);
        assert_eq!(w.feed(-0.2), 0);
        assert_eq!(w.feed(-0.9), -1);
    }

    /// Windows のホイールは 1 段で 3 行ぶん届く。代替スクロールでは 1 段を 1 行にする。
    #[test]
    fn alt_scroll_moves_one_line_per_notch_on_windows() {
        assert_eq!(alt_scroll_lines(3.0, true), 1.0);
        assert_eq!(alt_scroll_lines(-6.0, true), -2.0);
        assert_eq!(alt_scroll_lines(3.0, false), 3.0);
    }

    #[test]
    fn alt_scroll_arrows() {
        assert_eq!(alt_scroll_bytes(2, false), b"\x1b[A\x1b[A");
        assert_eq!(alt_scroll_bytes(-1, true), b"\x1bOB");
    }

    const SGR_DRAG: MouseMode = MouseMode { report: MouseReport::Drag, sgr: true };
    const NO_MODS: MouseMods = MouseMods { shift: false, alt: false, ctrl: false };

    #[test]
    fn mouse_off_sends_nothing() {
        let off = MouseMode { report: MouseReport::Off, sgr: true };
        assert!(!off.is_on());
        assert_eq!(mouse_bytes(off, MouseKey::Left, MouseAction::Press, 0, 0, NO_MODS), None);
    }

    #[test]
    fn sgr_press_release_and_wheel() {
        // 列と行は 1 始まりで送る。
        assert_eq!(mouse_bytes(SGR_DRAG, MouseKey::Left, MouseAction::Press, 4, 9, NO_MODS).unwrap(), b"\x1b[<0;10;5M");
        assert_eq!(mouse_bytes(SGR_DRAG, MouseKey::Left, MouseAction::Release, 4, 9, NO_MODS).unwrap(), b"\x1b[<0;10;5m");
        assert_eq!(mouse_bytes(SGR_DRAG, MouseKey::Right, MouseAction::Press, 0, 0, NO_MODS).unwrap(), b"\x1b[<2;1;1M");
        assert_eq!(mouse_bytes(SGR_DRAG, MouseKey::WheelUp, MouseAction::Press, 2, 3, NO_MODS).unwrap(), b"\x1b[<64;4;3M");
        assert_eq!(mouse_bytes(SGR_DRAG, MouseKey::WheelDown, MouseAction::Press, 2, 3, NO_MODS).unwrap(), b"\x1b[<65;4;3M");
        let mods = MouseMods { shift: true, alt: true, ctrl: true };
        assert_eq!(mouse_bytes(SGR_DRAG, MouseKey::Left, MouseAction::Press, 0, 0, mods).unwrap(), b"\x1b[<28;1;1M");
    }

    #[test]
    fn motion_depends_on_mode() {
        let click = MouseMode { report: MouseReport::Click, sgr: true };
        assert_eq!(mouse_bytes(click, MouseKey::Left, MouseAction::Move, 0, 0, NO_MODS), None);
        // ボタンを押したままの移動は Drag で送る（32 を足す）。
        assert_eq!(mouse_bytes(SGR_DRAG, MouseKey::Left, MouseAction::Move, 1, 1, NO_MODS).unwrap(), b"\x1b[<32;2;2M");
        // ボタン無しの移動は Motion のときだけ。
        assert_eq!(mouse_bytes(SGR_DRAG, MouseKey::None, MouseAction::Move, 1, 1, NO_MODS), None);
        let motion = MouseMode { report: MouseReport::Motion, sgr: true };
        assert_eq!(mouse_bytes(motion, MouseKey::None, MouseAction::Move, 1, 1, NO_MODS).unwrap(), b"\x1b[<35;2;2M");
    }

    #[test]
    fn x10_encoding() {
        let normal = MouseMode { report: MouseReport::Click, sgr: false };
        assert_eq!(mouse_bytes(normal, MouseKey::Left, MouseAction::Press, 4, 9, NO_MODS).unwrap(), b"\x1b[M\x20\x2a\x25");
        // 離したときは、どのボタンかを送らず 3 にする。
        assert_eq!(mouse_bytes(normal, MouseKey::Left, MouseAction::Release, 4, 9, NO_MODS).unwrap(), b"\x1b[M\x23\x2a\x25");
        assert_eq!(mouse_bytes(normal, MouseKey::WheelUp, MouseAction::Press, 0, 0, NO_MODS).unwrap(), b"\x1b[M\x60\x21\x21");
        // 1 バイトに収まらない位置は送らない。
        assert_eq!(mouse_bytes(normal, MouseKey::Left, MouseAction::Press, 0, 223, NO_MODS), None);
    }
}
