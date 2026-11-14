//! コマンドパレットの範囲（接頭辞）とコマンドの一覧（設計書 §5.9）。

use crate::keymap::{Command, Platform, context_bindings, default_bindings};

/// パレットで探す範囲。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// コマンド、プロジェクト、ターミナル、ファイルの横断（⌘K）。
    All,
    Commands,
    Files,
    Projects,
    /// 全文検索。
    Search,
}

/// 先頭の '>' '#' '@' で範囲を決め、残りの文字列（前後の空白を除く）を返す。接頭辞が無ければ base。
pub fn parse_input(input: &str, base: Scope) -> (Scope, &str) {
    let trimmed = input.trim_start();
    let scope = match trimmed.chars().next() {
        Some('>') => Scope::Commands,
        Some('#') => Scope::Search,
        Some('@') => Scope::Projects,
        _ => return (base, input.trim()),
    };
    let rest = &trimmed[1..];
    if scope == Scope::Search {
        // 全体検索は # の直後の空白 1 つだけを除き、前後の空白を残す（"  return" を探せる）。
        return (scope, rest.strip_prefix(' ').unwrap_or(rest));
    }
    (scope, rest.trim())
}

/// パレットから実行できるコマンド。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaletteCommand {
    NewTerminal,
    SplitTerminal,
    ToggleTree,
    Close,
    Save,
    OpenFolder,
    GoToFile,
    FindInFiles,
    FindInTerminal,
    NextTab,
    PrevTab,
}

impl PaletteCommand {
    pub const ALL: [PaletteCommand; 11] = [
        PaletteCommand::NewTerminal,
        PaletteCommand::SplitTerminal,
        PaletteCommand::ToggleTree,
        PaletteCommand::Close,
        PaletteCommand::Save,
        PaletteCommand::OpenFolder,
        PaletteCommand::GoToFile,
        PaletteCommand::FindInFiles,
        PaletteCommand::FindInTerminal,
        PaletteCommand::NextTab,
        PaletteCommand::PrevTab,
    ];

    /// 表示名。
    pub fn label(self) -> &'static str {
        match self {
            PaletteCommand::NewTerminal => "新しいターミナル",
            PaletteCommand::SplitTerminal => "ターミナルを分割",
            PaletteCommand::ToggleTree => "ファイルツリーの開閉",
            PaletteCommand::Close => "閉じる",
            PaletteCommand::Save => "保存",
            PaletteCommand::OpenFolder => "フォルダを開く",
            PaletteCommand::GoToFile => "ファイルを開く",
            PaletteCommand::FindInFiles => "全体検索",
            PaletteCommand::FindInTerminal => "ターミナル内を検索",
            PaletteCommand::NextTab => "次のタブ",
            PaletteCommand::PrevTab => "前のタブ",
        }
    }

    /// 英字で探すための別名（表示名は日本語のため）。
    pub fn keywords(self) -> &'static str {
        match self {
            PaletteCommand::NewTerminal => "new terminal tab",
            PaletteCommand::SplitTerminal => "split terminal",
            PaletteCommand::ToggleTree => "toggle file tree explorer",
            PaletteCommand::Close => "close",
            PaletteCommand::Save => "save",
            PaletteCommand::OpenFolder => "open folder project",
            PaletteCommand::GoToFile => "go to file open",
            PaletteCommand::FindInFiles => "find search in files grep",
            PaletteCommand::FindInTerminal => "find search in terminal",
            PaletteCommand::NextTab => "next tab",
            PaletteCommand::PrevTab => "previous tab",
        }
    }

    /// プロジェクトを開いていないと使えないか。
    pub fn needs_project(self) -> bool {
        self != PaletteCommand::OpenFolder
    }

    /// 割り当てたキー（keymap の Command）。無ければ None。
    pub fn keymap_command(self) -> Option<Command> {
        match self {
            PaletteCommand::NewTerminal => Some(Command::NewTab),
            PaletteCommand::SplitTerminal => Some(Command::SplitTab),
            PaletteCommand::ToggleTree => Some(Command::ToggleTree),
            PaletteCommand::Close => Some(Command::Close),
            PaletteCommand::Save => Some(Command::Save),
            PaletteCommand::NextTab => Some(Command::NextTab),
            PaletteCommand::PrevTab => Some(Command::PrevTab),
            PaletteCommand::GoToFile => Some(Command::GoToFile),
            PaletteCommand::FindInFiles => Some(Command::FindInFiles),
            PaletteCommand::FindInTerminal => Some(Command::FindInTerminal),
            PaletteCommand::OpenFolder => None,
        }
    }
}

/// GPUI のキー表記を表示用にする。"ctrl-shift-t" → "Ctrl+Shift+T"、"cmd-shift-f" → "⌘⇧F"、"cmd-}" → "⌘⇧]"。
pub fn key_label(key: &str, platform: Platform) -> String {
    let mac = platform == Platform::MacOs;
    let mut parts: Vec<String> = Vec::new();
    let mut pieces: Vec<&str> = key.split('-').collect();
    // "cmd--" のように、キー自体が "-" の場合。
    if key.ends_with("--") {
        pieces.truncate(pieces.len() - 2);
        pieces.push("-");
    }
    let Some((last, mods)) = pieces.split_last() else { return String::new() };
    let mut shift = false;
    for m in mods {
        let label = match (*m, mac) {
            ("cmd", true) => "⌘",
            ("cmd", false) => "Win",
            ("ctrl", true) => "⌃",
            ("ctrl", false) => "Ctrl",
            ("alt", true) => "⌥",
            ("alt", false) => "Alt",
            ("shift", true) => "⇧",
            ("shift", false) => "Shift",
            (other, _) => other,
        };
        shift |= *m == "shift";
        parts.push(label.to_string());
    }
    // GPUI は Shift と記号の組み合わせを、Shift を外した記号で渡す（keymap の説明を参照）。
    let key = match *last {
        "{" => Some("["),
        "}" => Some("]"),
        _ => None,
    };
    let key = match key {
        Some(k) => {
            if !shift {
                parts.push(if mac { "⇧".into() } else { "Shift".into() });
            }
            k.to_string()
        }
        None => match *last {
            "escape" => "Esc".into(),
            "enter" => "Enter".into(),
            k if k.chars().count() == 1 => k.to_uppercase(),
            k => {
                let mut c = k.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        },
    };
    parts.push(key);
    if mac { parts.concat() } else { parts.join("+") }
}

/// command に割り当てたキーのうち、文脈の無いもの（無ければ文脈付きのもの）の表示。
pub fn key_hint(command: Command, platform: Platform) -> Option<String> {
    let plain = default_bindings(platform).into_iter().find(|(_, c)| *c == command).map(|(k, _)| k);
    let key = plain.or_else(|| context_bindings(platform).into_iter().find(|(_, c, _)| *c == command).map(|(k, _, _)| k))?;
    Some(key_label(key, platform))
}

#[cfg(test)]
mod tests {
    use super::{PaletteCommand, Scope, key_hint, key_label, parse_input};
    use crate::keymap::{Command, Platform};

    #[test]
    fn prefixes_pick_scope() {
        assert_eq!(parse_input(">split", Scope::All), (Scope::Commands, "split"));
        // 全体検索は # の直後の空白 1 つだけを除き、前後の空白を残す（"  return" を探せる）。
        assert_eq!(parse_input("# foo ", Scope::All), (Scope::Search, "foo "));
        assert_eq!(parse_input("#  return", Scope::All), (Scope::Search, " return"));
        assert_eq!(parse_input("@web", Scope::Files), (Scope::Projects, "web"));
        assert_eq!(parse_input("main.rs", Scope::Files), (Scope::Files, "main.rs"));
        assert_eq!(parse_input("", Scope::All), (Scope::All, ""));
    }

    #[test]
    fn key_labels_per_platform() {
        assert_eq!(key_label("ctrl-shift-t", Platform::Windows), "Ctrl+Shift+T");
        assert_eq!(key_label("cmd-shift-f", Platform::MacOs), "⌘⇧F");
        assert_eq!(key_label("cmd-}", Platform::MacOs), "⌘⇧]");
        assert_eq!(key_label("ctrl-{", Platform::Windows), "Ctrl+Shift+[");
        assert_eq!(key_hint(Command::NewTab, Platform::Windows).as_deref(), Some("Ctrl+Shift+T"));
    }

    #[test]
    fn every_command_has_a_label() {
        for c in PaletteCommand::ALL {
            assert!(!c.label().is_empty());
        }
    }
}
