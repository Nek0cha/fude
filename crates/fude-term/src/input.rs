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

/// 代替スクロール。正（上）は上矢印、負は下矢印を |lines| 回送る。app_cursor なら `ESC O` 形式。
pub fn alt_scroll_bytes(lines: i32, app_cursor: bool) -> Vec<u8> {
    let arrow = if lines > 0 { b'A' } else { b'B' };
    let one = if app_cursor { [0x1b, b'O', arrow] } else { [0x1b, b'[', arrow] };
    one.repeat(lines.unsigned_abs() as usize)
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

    #[test]
    fn alt_scroll_arrows() {
        assert_eq!(alt_scroll_bytes(2, false), b"\x1b[A\x1b[A");
        assert_eq!(alt_scroll_bytes(-1, true), b"\x1bOB");
    }
}
