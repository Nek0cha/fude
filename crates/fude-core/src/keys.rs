//! キー入力を、PTY へ送るバイト列に変換する。

use crate::keymap::Platform;

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
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    /// ファンクションキー。1〜12。
    F(u8),
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
        KeyCode::Backspace if m.ctrl => Some(with_alt(&[0x08])),
        KeyCode::Backspace => Some(with_alt(&[0x7f])),
        KeyCode::Escape => Some(with_alt(&[0x1b])),
        KeyCode::Tab if m.shift => Some(with_alt(b"\x1b[Z")),
        KeyCode::Tab => Some(with_alt(b"\t")),
        KeyCode::Up => Some(arrow(b'A', m, modes)),
        KeyCode::Down => Some(arrow(b'B', m, modes)),
        KeyCode::Right => Some(arrow(b'C', m, modes)),
        KeyCode::Left => Some(arrow(b'D', m, modes)),
        KeyCode::Home => Some(arrow(b'H', m, modes)),
        KeyCode::End => Some(arrow(b'F', m, modes)),
        KeyCode::Insert => Some(tilde(2, m)),
        KeyCode::Delete => Some(tilde(3, m)),
        KeyCode::PageUp => Some(tilde(5, m)),
        KeyCode::PageDown => Some(tilde(6, m)),
        KeyCode::F(n @ 1..=4) => Some(ss3_function(b'P' + (n - 1), m)),
        KeyCode::F(n @ 5..=12) => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            Some(tilde(CODES[usize::from(n - 5)], m))
        }
        KeyCode::F(_) => None,
        KeyCode::Char(c) => {
            // 対応する制御文字が無い Ctrl+文字は、文字のまま送る。
            if let Some(b) = ctrl_byte(c).filter(|_| m.ctrl) {
                return Some(with_alt(&[b]));
            }
            let mut buf = [0u8; 4];
            Some(with_alt(c.encode_utf8(&mut buf).as_bytes()))
        }
    }
}

/// xterm の修飾の番号（修飾なしは 1）。
fn modifier_param(m: Modifiers) -> u8 {
    1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl)
}

/// `ESC [ code ~` 形式のキー。修飾キーがあるときは `ESC [ code ; n ~` にする。
fn tilde(code: u8, m: Modifiers) -> Vec<u8> {
    match modifier_param(m) {
        1 => format!("\x1b[{code}~").into_bytes(),
        param => format!("\x1b[{code};{param}~").into_bytes(),
    }
}

/// F1〜F4。修飾なしは `ESC O x`、修飾ありは `ESC [ 1 ; n x`。
fn ss3_function(final_byte: u8, m: Modifiers) -> Vec<u8> {
    match modifier_param(m) {
        1 => vec![0x1b, b'O', final_byte],
        param => format!("\x1b[1;{}{}", param, final_byte as char).into_bytes(),
    }
}

/// 矢印キー、Home、End の変換。修飾キーがあるときは xterm 形式（`ESC [ 1 ; n x`）にする。
fn arrow(final_byte: u8, m: Modifiers, modes: TermModes) -> Vec<u8> {
    let param = modifier_param(m);
    if param > 1 {
        format!("\x1b[1;{}{}", param, final_byte as char).into_bytes()
    } else if modes.app_cursor {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// Ctrl+文字を制御文字に変換する。数字と記号の対応は xterm に合わせる。対応が無ければ `None` を返す。
fn ctrl_byte(c: char) -> Option<u8> {
    match c {
        '@'..='_' => Some(c as u8 - b'@'),
        'a'..='z' => Some(c as u8 - b'a' + 1),
        ' ' | '2' => Some(0x00),
        '3' => Some(0x1b),
        '4' => Some(0x1c),
        '5' => Some(0x1d),
        '6' => Some(0x1e),
        '7' | '/' => Some(0x1f),
        '8' | '?' => Some(0x7f),
        _ => None,
    }
}

/// GPUI の Keystroke から、振り分けに必要な値だけを抜いたもの。
#[derive(Clone, Copy, Debug)]
pub struct RawKey<'a> {
    pub key: &'a str,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// macOS の ⌘、Windows の Win キー。
    pub platform: bool,
}

/// キー入力の行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRoute {
    /// encode_key でバイト列にして PTY に送る。
    Encode(KeyInput),
    /// 何もしない。文字は IME（EntityInputHandler）経由で届く。
    Text,
    /// 端末には送らない（⌘ / Win キー付き、修飾キー単体など）。
    Ignore,
}

/// キー入力を、端末へ直接送るか、IME の経路に任せるかに振り分ける。
/// macOS の Option は記号などの文字を作るため、Alt として送らない。
pub fn route_key(raw: &RawKey, os: Platform) -> KeyRoute {
    if raw.platform {
        return KeyRoute::Ignore;
    }
    let mods = Modifiers { shift: raw.shift, ctrl: raw.ctrl, alt: raw.alt };
    let encode = |code| KeyRoute::Encode(KeyInput { code, mods });
    let alt_as_meta = raw.alt && os == Platform::Windows;
    let named = match raw.key {
        "enter" => Some(KeyCode::Enter),
        "backspace" => Some(KeyCode::Backspace),
        "tab" => Some(KeyCode::Tab),
        "escape" => Some(KeyCode::Escape),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" => Some(KeyCode::PageUp),
        "pagedown" => Some(KeyCode::PageDown),
        "delete" => Some(KeyCode::Delete),
        "insert" => Some(KeyCode::Insert),
        key => key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=12).contains(n)).map(KeyCode::F),
    };
    if let Some(code) = named {
        return encode(code);
    }
    if raw.key == "space" {
        return if raw.ctrl || alt_as_meta { encode(KeyCode::Char(' ')) } else { KeyRoute::Text };
    }
    let mut chars = raw.key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return KeyRoute::Ignore };
    if raw.ctrl {
        encode(KeyCode::Char(c))
    } else if alt_as_meta {
        encode(KeyCode::Char(if raw.shift { c.to_ascii_uppercase() } else { c }))
    } else {
        KeyRoute::Text
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
    fn ctrl_space_and_symbols() {
        assert_eq!(enc(k(Char(' '), false, true, false)), [0x00]);
        assert_eq!(enc(k(Char('?'), false, true, false)), [0x7f]);
        assert_eq!(enc(k(Char('/'), false, true, false)), [0x1f]);
    }

    #[test]
    fn ctrl_digits_follow_xterm() {
        assert_eq!(enc(k(Char('2'), false, true, false)), [0x00]);
        assert_eq!(enc(k(Char('3'), false, true, false)), [0x1b]);
        assert_eq!(enc(k(Char('7'), false, true, false)), [0x1f]);
        assert_eq!(enc(k(Char('8'), false, true, false)), [0x7f]);
        assert_eq!(enc(k(Char('1'), false, true, false)), b"1");
    }

    #[test]
    fn ctrl_backspace_is_bs() {
        assert_eq!(enc(k(Backspace, false, true, false)), [0x08]);
    }

    #[test]
    fn alt_shift_tab_keeps_alt() {
        assert_eq!(enc(k(Tab, true, false, true)), b"\x1b\x1b[Z");
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

    #[test]
    fn home_end_follow_app_cursor() {
        assert_eq!(enc(k(Home, false, false, false)), b"\x1b[H");
        let app = TermModes { app_cursor: true };
        assert_eq!(encode_key(&k(Home, false, false, false), app, &KeyOverrides::defaults()).unwrap(), b"\x1bOH");
        assert_eq!(enc(k(End, false, true, false)), b"\x1b[1;5F");
    }

    #[test]
    fn tilde_keys() {
        assert_eq!(enc(k(Delete, false, false, false)), b"\x1b[3~");
        assert_eq!(enc(k(PageUp, true, false, false)), b"\x1b[5;2~");
        assert_eq!(enc(k(Insert, false, false, false)), b"\x1b[2~");
    }

    #[test]
    fn function_keys() {
        assert_eq!(enc(k(F(1), false, false, false)), b"\x1bOP");
        assert_eq!(enc(k(F(5), false, false, false)), b"\x1b[15~");
        assert_eq!(enc(k(F(12), false, false, false)), b"\x1b[24~");
        assert_eq!(enc(k(F(1), false, true, false)), b"\x1b[1;5P");
        assert_eq!(enc(k(F(5), true, false, false)), b"\x1b[15;2~");
    }

    fn raw(key: &str, shift: bool, ctrl: bool, alt: bool, platform: bool) -> RawKey<'_> {
        RawKey { key, shift, ctrl, alt, platform }
    }

    #[test]
    fn route_plain_char_is_text() {
        assert_eq!(route_key(&raw("a", false, false, false, false), Platform::Windows), KeyRoute::Text);
        assert_eq!(route_key(&raw("a", true, false, false, false), Platform::Windows), KeyRoute::Text);
    }

    #[test]
    fn route_ctrl_char_encodes() {
        assert_eq!(
            route_key(&raw("c", false, true, false, false), Platform::Windows),
            KeyRoute::Encode(k(Char('c'), false, true, false))
        );
    }

    #[test]
    fn route_alt_char_depends_on_os() {
        let alt_b = raw("b", false, false, true, false);
        assert_eq!(route_key(&alt_b, Platform::Windows), KeyRoute::Encode(k(Char('b'), false, false, true)));
        assert_eq!(route_key(&alt_b, Platform::MacOs), KeyRoute::Text);
        assert_eq!(
            route_key(&raw("b", true, false, true, false), Platform::Windows),
            KeyRoute::Encode(k(Char('B'), true, false, true))
        );
    }

    #[test]
    fn route_platform_is_ignored() {
        assert_eq!(route_key(&raw("c", false, false, false, true), Platform::MacOs), KeyRoute::Ignore);
    }

    #[test]
    fn route_named_keys_encode() {
        assert_eq!(
            route_key(&raw("enter", true, false, false, false), Platform::Windows),
            KeyRoute::Encode(k(Enter, true, false, false))
        );
        assert_eq!(route_key(&raw("f5", false, false, false, false), Platform::MacOs), KeyRoute::Encode(k(F(5), false, false, false)));
        assert_eq!(
            route_key(&raw("pageup", false, false, false, false), Platform::Windows),
            KeyRoute::Encode(k(PageUp, false, false, false))
        );
    }

    #[test]
    fn route_space() {
        assert_eq!(route_key(&raw("space", false, false, false, false), Platform::Windows), KeyRoute::Text);
        assert_eq!(
            route_key(&raw("space", false, true, false, false), Platform::Windows),
            KeyRoute::Encode(k(Char(' '), false, true, false))
        );
    }

    #[test]
    fn route_modifier_only_is_ignored() {
        assert_eq!(route_key(&raw("shift", true, false, false, false), Platform::Windows), KeyRoute::Ignore);
        assert_eq!(route_key(&raw("control", false, true, false, false), Platform::Windows), KeyRoute::Ignore);
    }
}
