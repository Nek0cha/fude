//! 設定の値と設定ファイル（設計書 §5.11、`2026-11-15-config-theme-design.md`）。
//! 読み込みは項目ごとに検査し、不正な項目だけを既定値にする。知らない項目名は黙って無視する。

use std::path::PathBuf;

use toml::{Table, Value};

use crate::keymap::Platform;

/// 最後のプロジェクトを閉じたときの動き。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastProjectClosed {
    /// ホーム画面に戻る。
    Home,
    /// アプリを終了する。
    Quit,
}

/// 同梱するテーマ（暗い版だけ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeId {
    Fude,
    Oled,
    GithubDark,
    GithubDarkDimmed,
    OneDarkPro,
    Dracula,
    TokyoNight,
    TokyoNightStorm,
    CatppuccinMocha,
    CatppuccinMacchiato,
    CatppuccinFrappe,
}

impl ThemeId {
    pub const ALL: [ThemeId; 11] = [
        ThemeId::Fude,
        ThemeId::Oled,
        ThemeId::GithubDark,
        ThemeId::GithubDarkDimmed,
        ThemeId::OneDarkPro,
        ThemeId::Dracula,
        ThemeId::TokyoNight,
        ThemeId::TokyoNightStorm,
        ThemeId::CatppuccinMocha,
        ThemeId::CatppuccinMacchiato,
        ThemeId::CatppuccinFrappe,
    ];

    /// 設定ファイルに書く名前。
    pub fn name(self) -> &'static str {
        match self {
            ThemeId::Fude => "fude",
            ThemeId::Oled => "oled",
            ThemeId::GithubDark => "github-dark",
            ThemeId::GithubDarkDimmed => "github-dark-dimmed",
            ThemeId::OneDarkPro => "one-dark-pro",
            ThemeId::Dracula => "dracula",
            ThemeId::TokyoNight => "tokyo-night",
            ThemeId::TokyoNightStorm => "tokyo-night-storm",
            ThemeId::CatppuccinMocha => "catppuccin-mocha",
            ThemeId::CatppuccinMacchiato => "catppuccin-macchiato",
            ThemeId::CatppuccinFrappe => "catppuccin-frappe",
        }
    }

    /// 設定パネルに出す名前。
    pub fn label(self) -> &'static str {
        match self {
            ThemeId::Fude => "fude",
            ThemeId::Oled => "OLED",
            ThemeId::GithubDark => "GitHub Dark",
            ThemeId::GithubDarkDimmed => "GitHub Dark Dimmed",
            ThemeId::OneDarkPro => "One Dark Pro",
            ThemeId::Dracula => "Dracula",
            ThemeId::TokyoNight => "Tokyo Night",
            ThemeId::TokyoNightStorm => "Tokyo Night Storm",
            ThemeId::CatppuccinMocha => "Catppuccin Mocha",
            ThemeId::CatppuccinMacchiato => "Catppuccin Macchiato",
            ThemeId::CatppuccinFrappe => "Catppuccin Frappé",
        }
    }

    pub fn from_name(name: &str) -> Option<ThemeId> {
        ThemeId::ALL.into_iter().find(|id| id.name() == name)
    }
}

/// ターミナルのカーソルの既定の形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

/// ローカルのプロジェクトで新しいターミナルに使うシェル。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeShell {
    /// Windows は pwsh.exe、無ければ powershell.exe。macOS は $SHELL。
    Auto,
    Pwsh,
    Powershell,
    Cmd,
    /// WSL のシェル（ディストリビューションは `wsl_distro`）。
    Wsl,
    /// 実行ファイルのパスか名前。
    Program(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSettings {
    /// ウィンドウの地の不透明度。1.0 で透かさない。
    pub background_opacity: f32,
    pub blur: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontSettings {
    /// UI の書体。None なら OS の既定。
    pub ui: Option<String>,
    /// ターミナルとエディタの書体。
    pub mono: String,
    /// ターミナルとエディタの文字の大きさ（px）。
    pub size: f32,
    /// ターミナルとエディタの行の高さ（px）。書かなければ size の 1.3 倍。
    pub line_height: f32,
    /// 行の高さを設定ファイルに書いたか（書いていなければ、文字の大きさに合わせて変わる）。
    pub line_height_explicit: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorSettings {
    pub shape: CursorShape,
    pub blink: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSettings {
    pub shell: NativeShell,
    /// None なら wsl.exe の既定のディストリビューション。
    pub wsl_distro: Option<String>,
    /// Shift+Enter で送るバイト列。
    pub shift_enter: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CloseSettings {
    /// 動いているプロセスがあるターミナルやプロジェクトを閉じるときに確認するか（§5.12）。
    pub confirm_running: bool,
    pub on_last_project: LastProjectClosed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreeSettings {
    pub show_heavy_dirs: bool,
    /// 書けば既定の隠すフォルダの一覧を置き換える。
    pub heavy_dirs: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateSettings {
    /// 1 日に 1 回、GitHub で新しい版を確かめるか。
    pub check: bool,
}

/// キー割り当ての上書き。Command の名前と、キーの一覧（書いた順）。中身の検査は keymap::bindings が行う。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct KeybindingOverrides {
    pub windows: Vec<(String, Vec<String>)>,
    pub macos: Vec<(String, Vec<String>)>,
    /// 書いた行（"windows.close" → 1 始まりの行）。問題の知らせに使う。
    pub lines: std::collections::HashMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: ThemeId,
    /// 書けばテーマのアクセント色より優先する（0xrrggbb）。
    pub accent: Option<u32>,
    pub window: WindowSettings,
    pub font: FontSettings,
    pub cursor: CursorSettings,
    pub terminal: TerminalSettings,
    pub close: CloseSettings,
    pub tree: TreeSettings,
    pub updates: UpdateSettings,
    pub keybindings: KeybindingOverrides,
}

pub const DEFAULT_OPACITY: f32 = 0.62;
pub const DEFAULT_MONO: &str = "HackGen Console NF";
pub const DEFAULT_FONT_SIZE: f32 = 14.0;

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeId::Fude,
            accent: None,
            window: WindowSettings { background_opacity: DEFAULT_OPACITY, blur: true },
            font: FontSettings { ui: None, mono: DEFAULT_MONO.into(), size: DEFAULT_FONT_SIZE, line_height: line_height_for(DEFAULT_FONT_SIZE), line_height_explicit: false },
            cursor: CursorSettings { shape: CursorShape::Block, blink: false },
            terminal: TerminalSettings { shell: NativeShell::Auto, wsl_distro: None, shift_enter: b"\x1b\r".to_vec() },
            close: CloseSettings { confirm_running: true, on_last_project: LastProjectClosed::Home },
            tree: TreeSettings { show_heavy_dirs: false, heavy_dirs: None },
            updates: UpdateSettings { check: true },
            keybindings: KeybindingOverrides::default(),
        }
    }
}

/// 行の高さを書かなかったときの値（文字の大きさの 1.3 倍）。
pub fn line_height_for(size: f32) -> f32 {
    (size * 1.3).round()
}

/// 設定ファイルの問題。line は 1 始まり。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigIssue {
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParseOutcome {
    /// 読めた。不正な項目は既定値にし、issues に積んだ。
    Ok { settings: Settings, issues: Vec<ConfigIssue> },
    /// TOML として読めない（空、書きかけを含む）。呼び出し側は直前の値を保つ。
    Unreadable(ConfigIssue),
}

/// 設定ファイルの置き場所。Windows は %APPDATA%\fude\config.toml、macOS は ~/.config/fude/config.toml。
/// FUDE_INSTANCE=x のときは config-x.toml。
pub fn config_path(os: Platform, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    crate::session::data_file(os, env, "config", "toml")
}

/// 設定ファイルが無いときに書き出す既定の文面。parse するとすべて既定値になる。
pub const DEFAULT_CONFIG: &str = r##"# fude の設定。保存するとすぐ反映されます。
# テーマ: fude / oled / github-dark / github-dark-dimmed / one-dark-pro / dracula /
#         tokyo-night / tokyo-night-storm / catppuccin-mocha / catppuccin-macchiato / catppuccin-frappe
theme = "fude"
# accent = "#b9a8ff"          # 書くとテーマのアクセント色より優先

[window]
background_opacity = 0.62     # 0.0〜1.0。1.0 で透かさない
blur = true

[font]
ui = ""                       # 空なら OS の既定
mono = "HackGen Console NF"   # ターミナルとエディタの書体
size = 14                     # ターミナルとエディタの文字の大きさ（8〜32）
# line_height = 18            # 書かなければ文字の大きさの 1.3 倍

[cursor]
shape = "block"               # "block" / "bar" / "underline"
blink = false

[terminal]
shell = ""                    # 空なら自動（Windows: pwsh、無ければ powershell。macOS: $SHELL）。"cmd"、"wsl"、パスも書ける
wsl_distro = ""               # 空なら wsl.exe の既定
shift_enter = "\u001b\r"

[close]
confirm_running = true        # 動いているターミナルを閉じるときに確認する
on_last_project = "home"      # "home" / "quit"

[tree]
show_heavy_dirs = false       # 隠すフォルダ（node_modules など）をツリーに薄く出す
# heavy_dirs = ["node_modules", "target"]   # 書くと既定の一覧を置き換える

[updates]
check = true                  # 1 日に 1 回、GitHub で新しい版を確かめる（送るのは問い合わせだけ）

[keybindings.windows]
# open_palette = ["ctrl-alt-p"]   # 書いた Command は既定のキーを置き換える。[] で外す

[keybindings.macos]
# open_palette = ["cmd-alt-p"]
"##;

/// 設定ファイルのバイト列を文字列にする。UTF-8（BOM 付きも）と、BOM 付きの UTF-16LE（古いメモ帳の「Unicode」）を読む。
/// それ以外は、何が悪いか分かる文の問題にする。
pub fn decode_config(bytes: &[u8]) -> Result<String, ConfigIssue> {
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16(&units).map_err(|_| ConfigIssue { line: None, message: "設定ファイルを UTF-8 で保存し直してください".into() });
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| ConfigIssue {
        line: None,
        message: "設定ファイルが UTF-8 ではありません。UTF-8 で保存し直してください".into(),
    })
}

/// 設定ファイルの文面を読む。
pub fn parse(text: &str) -> ParseOutcome {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim().is_empty() {
        return ParseOutcome::Unreadable(ConfigIssue { line: None, message: "設定ファイルが空です".into() });
    }
    let table: Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            let e: toml::de::Error = e;
            let line = e.span().map(|s| line_at(text, s.start));
            let message = format!("設定ファイルを読めません: {}", e.message().trim());
            return ParseOutcome::Unreadable(ConfigIssue { line, message });
        }
    };
    let mut r = Reader { text, issues: Vec::new() };
    let d = Settings::default();
    let root = Some(&table);
    let window = section(&table, "window");
    let font = section(&table, "font");
    let cursor = section(&table, "cursor");
    let terminal = section(&table, "terminal");
    let close = section(&table, "close");
    let tree = section(&table, "tree");
    let updates = section(&table, "updates");
    let keys = section(&table, "keybindings");

    let theme = r
        .string(root, None, "theme")
        .and_then(|(name, line)| r.check(ThemeId::from_name(&name), line, "theme", "テーマが見つかりません"))
        .unwrap_or(d.theme);
    let accent = r
        .string(root, None, "accent")
        .and_then(|(s, line)| r.check(parse_hex(&s), line, "accent", "\"#rrggbb\" の形で書いてください"));
    let background_opacity = r
        .number(window, Some("window"), "background_opacity")
        .and_then(|(v, line)| r.check(Some(v).filter(|v| (0.0..=1.0).contains(v)), line, "window.background_opacity", "0.0〜1.0 の数で書いてください"))
        .map_or(d.window.background_opacity, |v| v as f32);
    let blur = r.bool(window, Some("window"), "blur").unwrap_or(d.window.blur);

    let ui = r.string(font, Some("font"), "ui").map(|(s, _)| s).filter(|s| !s.is_empty());
    let mono = r.string(font, Some("font"), "mono").map(|(s, _)| s).filter(|s| !s.is_empty()).unwrap_or(d.font.mono);
    let size = r
        .number(font, Some("font"), "size")
        .and_then(|(v, line)| r.check(Some(v).filter(|v| (8.0..=32.0).contains(v)), line, "font.size", "8〜32 の数で書いてください"))
        .map_or(d.font.size, |v| v as f32);
    let line_height = r
        .number(font, Some("font"), "line_height")
        .and_then(|(v, line)| {
            let ok = Some(v).filter(|v| *v >= f64::from(size) && *v <= 64.0);
            r.check(ok, line, "font.line_height", "文字の大きさ以上、64 以下の数で書いてください")
        })
        .map(|v| v as f32);
    let line_height_explicit = line_height.is_some();
    let line_height = line_height.unwrap_or(line_height_for(size));

    let shape = r
        .string(cursor, Some("cursor"), "shape")
        .and_then(|(s, line)| {
            let shape = match s.as_str() {
                "block" => Some(CursorShape::Block),
                "bar" => Some(CursorShape::Bar),
                "underline" => Some(CursorShape::Underline),
                _ => None,
            };
            r.check(shape, line, "cursor.shape", "\"block\"、\"bar\"、\"underline\" のどれかで書いてください")
        })
        .unwrap_or(d.cursor.shape);
    let blink = r.bool(cursor, Some("cursor"), "blink").unwrap_or(d.cursor.blink);

    let shell = r.string(terminal, Some("terminal"), "shell").map_or(NativeShell::Auto, |(s, _)| match s.as_str() {
        "" => NativeShell::Auto,
        "pwsh" => NativeShell::Pwsh,
        "powershell" => NativeShell::Powershell,
        "cmd" => NativeShell::Cmd,
        "wsl" => NativeShell::Wsl,
        _ => NativeShell::Program(s),
    });
    let wsl_distro = r.string(terminal, Some("terminal"), "wsl_distro").map(|(s, _)| s).filter(|s| !s.is_empty());
    let shift_enter = r.string(terminal, Some("terminal"), "shift_enter").map_or(d.terminal.shift_enter, |(s, _)| s.into_bytes());

    let confirm_running = r.bool(close, Some("close"), "confirm_running").unwrap_or(d.close.confirm_running);
    let on_last_project = r
        .string(close, Some("close"), "on_last_project")
        .and_then(|(s, line)| {
            let v = match s.as_str() {
                "home" => Some(LastProjectClosed::Home),
                "quit" => Some(LastProjectClosed::Quit),
                _ => None,
            };
            r.check(v, line, "close.on_last_project", "\"home\" か \"quit\" で書いてください")
        })
        .unwrap_or(d.close.on_last_project);

    let show_heavy_dirs = r.bool(tree, Some("tree"), "show_heavy_dirs").unwrap_or(d.tree.show_heavy_dirs);
    let heavy_dirs = r.string_list(tree, Some("tree"), "heavy_dirs");

    let check = r.bool(updates, Some("updates"), "check").unwrap_or(d.updates.check);

    let mut keybindings = KeybindingOverrides {
        windows: r.key_table(keys, "keybindings.windows", "windows"),
        macos: r.key_table(keys, "keybindings.macos", "macos"),
        lines: Default::default(),
    };
    for (os, rows) in [("windows", &keybindings.windows), ("macos", &keybindings.macos)] {
        for (name, _) in rows {
            if let Some(line) = line_of(text, Some(&format!("keybindings.{os}")), name) {
                keybindings.lines.insert(format!("{os}.{name}"), line);
            }
        }
    }
    for name in ["window", "font", "cursor", "terminal", "close", "tree", "updates", "keybindings"] {
        if table.get(name).is_some_and(|v| !v.is_table()) {
            let line = line_of(text, None, name);
            r.issue(line, name, &format!("表（[{name}]）で書いてください"));
        }
    }

    let settings = Settings {
        theme,
        accent,
        window: WindowSettings { background_opacity, blur },
        font: FontSettings { ui, mono, size, line_height, line_height_explicit },
        cursor: CursorSettings { shape, blink },
        terminal: TerminalSettings { shell, wsl_distro, shift_enter },
        close: CloseSettings { confirm_running, on_last_project },
        tree: TreeSettings { show_heavy_dirs, heavy_dirs },
        updates: UpdateSettings { check },
        keybindings,
    };
    ParseOutcome::Ok { settings, issues: r.issues }
}

fn section<'a>(table: &'a Table, name: &str) -> Option<&'a Table> {
    table.get(name).and_then(Value::as_table)
}

/// "#rrggbb" → 0xrrggbb。
fn parse_hex(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten()
}

/// バイト位置の行番号（1 始まり）。
fn line_at(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())].iter().filter(|&&b| b == b'\n').count() + 1
}

/// 表 section（None は先頭の表）の中で key を書いた行（1 始まり）。単純な `[名前]` の見出しだけを見る。
fn line_of(text: &str, section: Option<&str>, key: &str) -> Option<usize> {
    let mut current: Option<String> = None;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if let Some(inner) = t.strip_prefix('[').and_then(|r| r.split(']').next()) {
            current = Some(inner.trim().to_string());
            continue;
        }
        // 先頭の表に `section.key = ...` とドット付きで書いた形も探す。
        let dotted = section.filter(|_| current.is_none()).map(|s| format!("{s}.{key}"));
        if let Some(rest) = dotted.as_deref().and_then(|d| t.strip_prefix(d))
            && rest.trim_start().starts_with('=')
        {
            return Some(i + 1);
        }
        if current.as_deref() != section {
            continue;
        }
        let rest = t.strip_prefix(key).or_else(|| t.strip_prefix(&format!("\"{key}\"")));
        if rest.is_some_and(|r| r.trim_start().starts_with('=')) {
            return Some(i + 1);
        }
    }
    None
}

struct Reader<'a> {
    text: &'a str,
    issues: Vec<ConfigIssue>,
}

impl Reader<'_> {
    fn issue(&mut self, line: Option<usize>, name: &str, message: &str) {
        self.issues.push(ConfigIssue { line, message: format!("{name} は{message}") });
    }

    /// value が None なら issue を積む。
    fn check<T>(&mut self, value: Option<T>, line: Option<usize>, name: &str, message: &str) -> Option<T> {
        if value.is_none() {
            self.issue(line, name, message);
        }
        value
    }

    /// 項目の値と行。項目が無ければ None。
    fn get<'t>(&self, table: Option<&'t Table>, section: Option<&str>, key: &str) -> Option<(&'t Value, Option<usize>)> {
        let value = table?.get(key)?;
        Some((value, line_of(self.text, section, key)))
    }

    fn full_name(section: Option<&str>, key: &str) -> String {
        section.map_or_else(|| key.to_string(), |s| format!("{s}.{key}"))
    }

    fn string(&mut self, table: Option<&Table>, section: Option<&str>, key: &str) -> Option<(String, Option<usize>)> {
        let (value, line) = self.get(table, section, key)?;
        match value.as_str() {
            Some(s) => Some((s.to_string(), line)),
            None => {
                self.issue(line, &Self::full_name(section, key), "文字列（\"...\"）で書いてください");
                None
            }
        }
    }

    fn number(&mut self, table: Option<&Table>, section: Option<&str>, key: &str) -> Option<(f64, Option<usize>)> {
        let (value, line) = self.get(table, section, key)?;
        match value {
            Value::Float(f) => Some((*f, line)),
            Value::Integer(i) => Some((*i as f64, line)),
            _ => {
                self.issue(line, &Self::full_name(section, key), "数で書いてください");
                None
            }
        }
    }

    fn bool(&mut self, table: Option<&Table>, section: Option<&str>, key: &str) -> Option<bool> {
        let (value, line) = self.get(table, section, key)?;
        match value.as_bool() {
            Some(b) => Some(b),
            None => {
                self.issue(line, &Self::full_name(section, key), "true か false で書いてください");
                None
            }
        }
    }

    fn string_list(&mut self, table: Option<&Table>, section: Option<&str>, key: &str) -> Option<Vec<String>> {
        let (value, line) = self.get(table, section, key)?;
        let list = value.as_array().and_then(|a| a.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<Vec<_>>>());
        self.check(list, line, &Self::full_name(section, key), "文字列の配列（[\"...\"]）で書いてください")
    }

    /// [keybindings.<os>] の表。値が文字列の配列でない行は捨てて issue を積む。
    fn key_table(&mut self, keys: Option<&Table>, section: &str, os: &str) -> Vec<(String, Vec<String>)> {
        let Some(table) = keys.and_then(|k| k.get(os)).and_then(Value::as_table) else { return Vec::new() };
        let mut out = Vec::new();
        for name in table.keys() {
            if let Some(list) = self.string_list(Some(table), Some(section), name) {
                out.push((name.clone(), list));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Platform;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn ok(text: &str) -> (Settings, Vec<ConfigIssue>) {
        match parse(text) {
            ParseOutcome::Ok { settings, issues } => (settings, issues),
            ParseOutcome::Unreadable(issue) => panic!("読めない: {issue:?}"),
        }
    }

    #[test]
    fn empty_text_is_unreadable() {
        assert!(matches!(parse(""), ParseOutcome::Unreadable(_)));
        assert!(matches!(parse("  \r\n"), ParseOutcome::Unreadable(_)));
    }

    #[test]
    fn default_config_parses_to_defaults_without_issues() {
        let (settings, issues) = ok(DEFAULT_CONFIG);
        assert_eq!(settings, Settings::default());
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let (settings, issues) = ok("theme = \"dracula\"\n");
        assert!(issues.is_empty());
        assert_eq!(settings, Settings { theme: ThemeId::Dracula, ..Settings::default() });
    }

    #[test]
    fn broken_toml_is_unreadable_with_line() {
        let ParseOutcome::Unreadable(issue) = parse("theme = ") else { panic!() };
        assert_eq!(issue.line, Some(1));
    }

    #[test]
    fn bad_value_falls_back_per_item() {
        let (settings, issues) = ok("theme = \"nope\"\n[window]\nbackground_opacity = 2.0\nblur = false\n");
        assert_eq!(settings.theme, ThemeId::Fude);
        assert_eq!(settings.window.background_opacity, 0.62);
        assert!(!settings.window.blur);
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert_eq!(issues[0].line, Some(1));
        assert_eq!(issues[1].line, Some(3));
    }

    #[test]
    fn integer_opacity_is_accepted_string_is_not() {
        let (settings, issues) = ok("[window]\nbackground_opacity = 1\n");
        assert_eq!(settings.window.background_opacity, 1.0);
        assert!(issues.is_empty());
        let (settings, issues) = ok("[window]\nbackground_opacity = \"0.5\"\n");
        assert_eq!(settings.window.background_opacity, 0.62);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn unknown_keys_are_ignored_silently() {
        let (_, issues) = ok("future_key = 1\n[window]\nnew_thing = true\n[future_table]\nx = 1\n");
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn bom_and_crlf_are_read() {
        let (settings, issues) = ok("\u{feff}theme = \"oled\"\r\n[font]\r\nsize = 16\r\n");
        assert!(issues.is_empty());
        assert_eq!(settings.theme, ThemeId::Oled);
        assert_eq!(settings.font.size, 16.0);
    }

    #[test]
    fn line_height_defaults_to_size_ratio() {
        assert_eq!(ok("[font]\nsize = 20\n").0.font.line_height, 26.0);
        assert_eq!(ok("[font]\nsize = 14\n").0.font.line_height, 18.0);
        assert_eq!(ok("[font]\nsize = 14\nline_height = 22\n").0.font.line_height, 22.0);
        let (settings, issues) = ok("[font]\nsize = 14\nline_height = 10\n");
        assert_eq!(settings.font.line_height, 18.0);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn font_size_out_of_range_falls_back() {
        let (settings, issues) = ok("[font]\nsize = 40\n");
        assert_eq!(settings.font.size, 14.0);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn line_height_explicit_only_when_written() {
        assert!(!ok("[font]\nsize = 14\n").0.font.line_height_explicit);
        assert!(ok("[font]\nsize = 14\nline_height = 22\n").0.font.line_height_explicit);
        assert!(!ok("[font]\nsize = 14\nline_height = 10\n").0.font.line_height_explicit);
        assert!(!Settings::default().font.line_height_explicit);
    }

    #[test]
    fn every_theme_has_label() {
        let labels: std::collections::HashSet<&str> = ThemeId::ALL.iter().map(|t| t.label()).collect();
        assert_eq!(labels.len(), ThemeId::ALL.len());
        assert!(labels.iter().all(|l| !l.is_empty()));
        assert_eq!(ThemeId::CatppuccinFrappe.label(), "Catppuccin Frappé");
    }

    #[test]
    fn accent_parses_hex() {
        assert_eq!(ok("accent = \"#ff0080\"\n").0.accent, Some(0xff0080));
        let (settings, issues) = ok("accent = \"red\"\n");
        assert_eq!(settings.accent, None);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn shift_enter_is_bytes() {
        assert_eq!(Settings::default().terminal.shift_enter, b"\x1b\r".to_vec());
        assert_eq!(ok("[terminal]\nshift_enter = \"\\\\\\r\"\n").0.terminal.shift_enter, b"\\\r".to_vec());
    }

    #[test]
    fn shell_names() {
        assert_eq!(ok("[terminal]\nshell = \"cmd\"\n").0.terminal.shell, NativeShell::Cmd);
        assert_eq!(ok("[terminal]\nshell = \"wsl\"\nwsl_distro = \"Ubuntu\"\n").0.terminal.wsl_distro.as_deref(), Some("Ubuntu"));
        assert_eq!(ok("[terminal]\nshell = \"/bin/fish\"\n").0.terminal.shell, NativeShell::Program("/bin/fish".into()));
        assert_eq!(ok("[terminal]\nshell = \"\"\n").0.terminal.shell, NativeShell::Auto);
    }

    #[test]
    fn close_and_cursor() {
        let (s, issues) = ok("[close]\nconfirm_running = false\non_last_project = \"quit\"\n[cursor]\nshape = \"bar\"\nblink = true\n");
        assert!(issues.is_empty());
        assert!(!s.close.confirm_running);
        assert_eq!(s.close.on_last_project, LastProjectClosed::Quit);
        assert_eq!(s.cursor.shape, CursorShape::Bar);
        assert!(s.cursor.blink);
    }

    #[test]
    fn tree_and_keybindings() {
        let text = "[tree]\nshow_heavy_dirs = true\nheavy_dirs = [\"target-dev\"]\n[keybindings.windows]\nclose = [\"ctrl-q\"]\ntoggle_tree = []\n[keybindings.macos]\nclose = [\"cmd-q\"]\n";
        let (s, issues) = ok(text);
        assert!(issues.is_empty(), "{issues:?}");
        assert!(s.tree.show_heavy_dirs);
        assert_eq!(s.tree.heavy_dirs, Some(vec!["target-dev".to_string()]));
        assert_eq!(s.keybindings.windows, vec![("close".to_string(), vec!["ctrl-q".to_string()]), ("toggle_tree".to_string(), vec![])]);
        assert_eq!(s.keybindings.macos, vec![("close".to_string(), vec!["cmd-q".to_string()])]);
    }

    #[test]
    fn updates_check() {
        let (s, issues) = ok("[updates]
check = false
");
        assert!(issues.is_empty(), "{issues:?}");
        assert!(!s.updates.check);
        assert!(ok("theme = \"fude\"
").0.updates.check);
        let (s, issues) = ok("[updates]
check = 1
");
        assert!(s.updates.check);
        assert_eq!(issues.len(), 1, "{issues:?}");
        let (_, issues) = ok("updates = 3
");
        assert_eq!(issues.len(), 1, "{issues:?}");
    }

    #[test]
    fn line_at_never_panics_inside_a_character() {
        let text = "a = \"あ\"\nb = 1\n";
        let inside = text.find('あ').unwrap() + 1;
        assert_eq!(line_at(text, inside), 1);
        assert_eq!(line_at(text, text.len() + 10), 3);
    }

    #[test]
    fn dotted_key_issue_has_line() {
        let (_, issues) = ok("theme = \"fude\"\nwindow.background_opacity = 2.0\n");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, Some(2));
    }

    #[test]
    fn section_that_is_not_a_table_is_an_issue() {
        let (_, issues) = ok("window = 3\n");
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].line, Some(1));
    }

    #[test]
    fn keybinding_lines_are_kept() {
        let (s, _) = ok("[keybindings.windows]\n\nnope = [\"ctrl-1\"]\n");
        assert_eq!(s.keybindings.lines.get("windows.nope"), Some(&3));
    }

    #[test]
    fn decode_reads_utf8_bom_and_utf16() {
        assert_eq!(decode_config(b"theme = 1").unwrap(), "theme = 1");
        let mut utf16 = vec![0xff, 0xfe];
        for u in "theme = \"oled\"".encode_utf16() {
            utf16.extend(u.to_le_bytes());
        }
        assert_eq!(decode_config(&utf16).unwrap(), "theme = \"oled\"");
        // Shift-JIS の「設定」。
        let issue = decode_config(&[0x90, 0xdd, 0x92, 0xe8]).unwrap_err();
        assert!(issue.message.contains("UTF-8"), "{issue:?}");
    }

    #[test]
    fn theme_names_round_trip() {
        for id in ThemeId::ALL {
            assert_eq!(ThemeId::from_name(id.name()), Some(id));
        }
        assert_eq!(ThemeId::ALL.len(), 11);
    }

    #[test]
    fn config_path_follows_instance() {
        let path = |os, vars: &[(&str, &str)]| {
            let map: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            config_path(os, &|k| map.get(k).cloned())
        };
        assert_eq!(path(Platform::Windows, &[("APPDATA", r"C:\A")]), Some(Path::new(r"C:\A").join("fude").join("config.toml")));
        assert_eq!(
            path(Platform::Windows, &[("APPDATA", r"C:\A"), ("FUDE_INSTANCE", "dev")]),
            Some(Path::new(r"C:\A").join("fude").join("config-dev.toml"))
        );
        assert_eq!(path(Platform::MacOs, &[("HOME", "/Users/u")]), Some(PathBuf::from("/Users/u").join(".config").join("fude").join("config.toml")));
    }
}
