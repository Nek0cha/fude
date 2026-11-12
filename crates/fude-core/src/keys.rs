//! キー入力を、PTY へ送るバイト列に変換する。

/// 変換の対象とするキー。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Right,
    Left,
    /// 文字キー。Shift による大文字化は、呼び出し側で済ませた文字を渡す。
    Char(char),
}

/// 修飾キーの状態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// 1 回のキー入力。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub code: KeyCode,
    pub mods: Modifiers,
}

/// 変換に影響するターミナルのモード。
#[derive(Clone, Copy, Debug, Default)]
pub struct TermModes {
    /// アプリケーションカーソルモード（DECCKM）。有効なときは矢印キーを `ESC O x` で送る。
    pub app_cursor: bool,
}

/// 特定のキー入力に対して、既定の変換の代わりに送るバイト列。
#[derive(Clone, Debug, Default)]
pub struct KeyOverrides {
    entries: Vec<(KeyInput, Vec<u8>)>,
}

impl KeyOverrides {
    /// 上書きを持たない。
    pub fn empty() -> Self {
        Self::default()
    }

    /// fude の既定値。Shift+Enter で `ESC CR` を送り、Claude Code の入力欄で改行できるようにする。
    pub fn defaults() -> Self {
        let mut o = Self::empty();
        o.insert(
            KeyInput { code: KeyCode::Enter, mods: Modifiers { shift: true, ..Modifiers::default() } },
            b"\x1b\r".to_vec(),
        );
        o
    }

    /// 上書きを追加する。同じキー入力がすでにあれば置き換える。
    pub fn insert(&mut self, key: KeyInput, bytes: Vec<u8>) {
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = bytes,
            None => self.entries.push((key, bytes)),
        }
    }

    fn get(&self, key: &KeyInput) -> Option<&[u8]> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, b)| b.as_slice())
    }
}

/// キー入力を PTY へ送るバイト列に変換する。送るものが無いキーは `None` を返す。
pub fn encode_key(input: &KeyInput, modes: TermModes, overrides: &KeyOverrides) -> Option<Vec<u8>> {
    if let Some(bytes) = overrides.get(input) {
        return Some(bytes.to_vec());
    }
    let m = input.mods;
    let with_alt = |bytes: &[u8]| {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if m.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(bytes);
        out
    };
    match input.code {
        KeyCode::Enter => Some(with_alt(b"\r")),
        KeyCode::Backspace => Some(with_alt(&[0x7f])),
        KeyCode::Escape => Some(with_alt(&[0x1b])),
        KeyCode::Tab if m.shift => Some(b"\x1b[Z".to_vec()),
        KeyCode::Tab => Some(with_alt(b"\t")),
        KeyCode::Up => Some(arrow(b'A', m, modes)),
        KeyCode::Down => Some(arrow(b'B', m, modes)),
        KeyCode::Right => Some(arrow(b'C', m, modes)),
        KeyCode::Left => Some(arrow(b'D', m, modes)),
        KeyCode::Char(c) if m.ctrl => ctrl_byte(c).map(|b| with_alt(&[b])),
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            Some(with_alt(c.encode_utf8(&mut buf).as_bytes()))
        }
    }
}

/// 矢印キーの変換。修飾キーがあるときは xterm 形式（`ESC [ 1 ; n x`）にする。
fn arrow(final_byte: u8, m: Modifiers, modes: TermModes) -> Vec<u8> {
    let param = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
    if param > 1 {
        format!("\x1b[1;{}{}", param, final_byte as char).into_bytes()
    } else if modes.app_cursor {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// Ctrl+文字を制御文字に変換する。`@`〜`_` と `a`〜`z` だけを対象とする。
fn ctrl_byte(c: char) -> Option<u8> {
    match c {
        '@'..='_' => Some(c as u8 - b'@'),
        'a'..='z' => Some(c as u8 - b'a' + 1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::KeyCode::*;
    use super::*;

    fn k(code: KeyCode, shift: bool, ctrl: bool, alt: bool) -> KeyInput {
        KeyInput { code, mods: Modifiers { shift, ctrl, alt } }
    }

    fn enc(input: KeyInput) -> Vec<u8> {
        encode_key(&input, TermModes::default(), &KeyOverrides::defaults()).unwrap()
    }

    #[test]
    fn enter_is_cr() {
        assert_eq!(enc(k(Enter, false, false, false)), b"\r");
    }

    #[test]
    fn shift_enter_default_is_esc_cr() {
        assert_eq!(enc(k(Enter, true, false, false)), b"\x1b\r");
    }

    #[test]
    fn shift_enter_without_override_is_cr() {
        let out = encode_key(&k(Enter, true, false, false), TermModes::default(), &KeyOverrides::empty()).unwrap();
        assert_eq!(out, b"\r");
    }

    #[test]
    fn override_replaces_bytes() {
        let mut o = KeyOverrides::defaults();
        o.insert(k(Enter, true, false, false), b"\\\r".to_vec());
        assert_eq!(encode_key(&k(Enter, true, false, false), TermModes::default(), &o).unwrap(), b"\\\r");
    }

    #[test]
    fn alt_enter_is_esc_cr() {
        assert_eq!(enc(k(Enter, false, false, true)), b"\x1b\r");
    }

    #[test]
    fn backspace_is_del() {
        assert_eq!(enc(k(Backspace, false, false, false)), [0x7f]);
    }

    #[test]
    fn tab_and_backtab() {
        assert_eq!(enc(k(Tab, false, false, false)), b"\t");
        assert_eq!(enc(k(Tab, true, false, false)), b"\x1b[Z");
    }

    #[test]
    fn escape_is_esc() {
        assert_eq!(enc(k(Escape, false, false, false)), [0x1b]);
    }

    #[test]
    fn arrows_normal_and_app_cursor() {
        assert_eq!(enc(k(Up, false, false, false)), b"\x1b[A");
        let app = TermModes { app_cursor: true };
        assert_eq!(encode_key(&k(Up, false, false, false), app, &KeyOverrides::defaults()).unwrap(), b"\x1bOA");
    }

    #[test]
    fn modified_arrow_uses_xterm_param() {
        assert_eq!(enc(k(Up, false, true, false)), b"\x1b[1;5A");
        assert_eq!(enc(k(Left, true, false, true)), b"\x1b[1;4D");
    }

    #[test]
    fn ctrl_letters() {
        assert_eq!(enc(k(Char('c'), false, true, false)), [0x03]);
        assert_eq!(enc(k(Char('a'), false, true, false)), [0x01]);
        assert_eq!(enc(k(Char('['), false, true, false)), [0x1b]);
    }

    #[test]
    fn alt_char_prefixes_esc() {
        assert_eq!(enc(k(Char('b'), false, false, true)), b"\x1bb");
    }

    #[test]
    fn shift_char_is_plain_text() {
        assert_eq!(enc(k(Char('A'), true, false, false)), b"A");
    }

    #[test]
    fn cjk_char_is_utf8() {
        assert_eq!(enc(k(Char('あ'), false, false, false)), "あ".as_bytes());
    }
}
