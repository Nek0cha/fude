//! プラットフォームごとのショートカットの割り当て表（設計書 §10）。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") { Platform::MacOs } else { Platform::Windows }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    NewTab,
    SplitTab,
    NextTab,
    PrevTab,
    /// 1〜9。
    ActivateProject(u8),
    /// ターミナルの選択範囲をコピーする。ターミナルの中だけで効く。
    Copy,
    /// クリップボードの文字列をターミナルに貼り付ける。ターミナルの中だけで効く。
    Paste,
    /// 閉じる（エディタのタブ、ツリー、ペイン、タブ、プロジェクトの順。設計書 §5.12）。
    Close,
    /// ファイルツリーの開閉。
    ToggleTree,
    /// エディタ列のファイルを保存する。
    Save,
    /// コマンドパレットを開く。
    OpenPalette,
    /// ファイル検索に絞ったパレットを開く。
    GoToFile,
    /// 全体検索のパレットを開く。
    FindInFiles,
    /// ターミナル内の検索欄を開く。ターミナルの中だけで効く。
    FindInTerminal,
    /// 路線図の折りたたみの切り替え。
    ToggleRouteMap,
    /// 開いているファイル（選んでいれば行の範囲）を、最後に使ったターミナルに `@` で入力する。ファイルの中だけで効く。
    SendToClaude,
}

/// ターミナル以外にフォーカスがあるときだけ効かせる文脈。
pub const NOT_TERMINAL: &str = "Workspace && !Terminal";
/// エディタ列のファイルの中だけで効かせる文脈。
pub const FILE_VIEW: &str = "FileView";
/// ターミナルの中だけで効かせる文脈。
pub const TERMINAL: &str = "Terminal";
/// gpui-kit の入力欄（エディタも含む）の文脈。
pub const INPUT: &str = "Input";

const PROJECT_KEYS_MAC: [&str; 9] = ["cmd-1", "cmd-2", "cmd-3", "cmd-4", "cmd-5", "cmd-6", "cmd-7", "cmd-8", "cmd-9"];
const PROJECT_KEYS_WIN: [&str; 9] =
    ["ctrl-1", "ctrl-2", "ctrl-3", "ctrl-4", "ctrl-5", "ctrl-6", "ctrl-7", "ctrl-8", "ctrl-9"];

/// GPUI のキーストローク表記（"cmd-t" など）と Command の組の一覧。
/// Windows では、ターミナルの Ctrl+T / Ctrl+D と衝突しないよう Shift 付きにする。
/// Shift と記号の組み合わせは、GPUI が受け取る形（Shift を外し、記号を `{` `}` にした形）で書く。
pub fn default_bindings(platform: Platform) -> Vec<(&'static str, Command)> {
    let (fixed, projects) = match platform {
        Platform::MacOs => (
            [
                ("cmd-t", Command::NewTab),
                ("cmd-d", Command::SplitTab),
                ("cmd-}", Command::NextTab),
                ("cmd-{", Command::PrevTab),
                ("cmd-c", Command::Copy),
                ("cmd-v", Command::Paste),
                ("cmd-w", Command::Close),
                ("cmd-b", Command::ToggleTree),
                ("cmd-k", Command::OpenPalette),
                ("cmd-p", Command::GoToFile),
                ("cmd-shift-f", Command::FindInFiles),
                ("cmd-\\", Command::ToggleRouteMap),
            ],
            PROJECT_KEYS_MAC,
        ),
        Platform::Windows => (
            [
                ("ctrl-shift-t", Command::NewTab),
                ("ctrl-shift-d", Command::SplitTab),
                ("ctrl-}", Command::NextTab),
                ("ctrl-{", Command::PrevTab),
                // Ctrl+V は Claude Code の画像の貼り付けのため、端末へそのまま送る。
                ("ctrl-shift-c", Command::Copy),
                ("ctrl-shift-v", Command::Paste),
                // Ctrl+W と Ctrl+B はシェルのキー操作と重なるため、どこでも効くのは Shift 付きだけ（§10）。
                ("ctrl-shift-w", Command::Close),
                ("ctrl-shift-b", Command::ToggleTree),
                // Ctrl+K / Ctrl+P もシェルのキー操作と重なるため、どこでも効くのは Shift 付きだけ（§10）。
                ("ctrl-shift-k", Command::OpenPalette),
                ("ctrl-shift-p", Command::GoToFile),
                ("ctrl-shift-f", Command::FindInFiles),
                ("ctrl-\\", Command::ToggleRouteMap),
            ],
            PROJECT_KEYS_WIN,
        ),
    };
    let projects = projects.into_iter().zip(1u8..).map(|(k, n)| (k, Command::ActivateProject(n)));
    fixed.into_iter().chain(projects).collect()
}

/// 文脈を限って効かせるキー。履歴は設計書の ⌘[ / ⌘] ではなく、エディタの字下げと重ならないキーにした（計画5）。
pub fn context_bindings(platform: Platform) -> Vec<(&'static str, Command, &'static str)> {
    match platform {
        Platform::MacOs => vec![
            ("cmd-s", Command::Save, FILE_VIEW),
            ("cmd-shift-l", Command::SendToClaude, FILE_VIEW),
            ("cmd-f", Command::FindInTerminal, TERMINAL),
            // gpui-kit の入力欄は ⌘⇧F を置換に割り当てているため、全体検索を優先させる（置換は ⌘⌥F）。
            ("cmd-shift-f", Command::FindInFiles, INPUT),
        ],
        Platform::Windows => vec![
            ("ctrl-w", Command::Close, NOT_TERMINAL),
            ("ctrl-b", Command::ToggleTree, NOT_TERMINAL),
            ("ctrl-s", Command::Save, FILE_VIEW),
            ("ctrl-shift-l", Command::SendToClaude, FILE_VIEW),
            ("ctrl-k", Command::OpenPalette, NOT_TERMINAL),
            ("ctrl-p", Command::GoToFile, NOT_TERMINAL),
            // Ctrl+F はシェル（1 文字進む、fish の補完の確定）に、Ctrl+Shift+F は全体検索に使う。
            ("ctrl-alt-f", Command::FindInTerminal, TERMINAL),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const BOTH: [Platform; 2] = [Platform::MacOs, Platform::Windows];

    #[test]
    fn route_map_key() {
        assert!(default_bindings(Platform::MacOs).contains(&("cmd-\\", Command::ToggleRouteMap)));
        assert!(default_bindings(Platform::Windows).contains(&("ctrl-\\", Command::ToggleRouteMap)));
    }

    #[test]
    fn mac_new_tab_is_cmd_t() {
        assert!(default_bindings(Platform::MacOs).contains(&("cmd-t", Command::NewTab)));
    }

    #[test]
    fn windows_new_tab_is_ctrl_shift_t() {
        assert!(default_bindings(Platform::Windows).contains(&("ctrl-shift-t", Command::NewTab)));
    }

    /// GPUI は Shift と記号の組み合わせを、Shift を外した記号（`{` `}`）で受け取る。
    #[test]
    fn tab_cycling_uses_shifted_bracket_chars() {
        let mac = default_bindings(Platform::MacOs);
        assert!(mac.contains(&("cmd-}", Command::NextTab)));
        assert!(mac.contains(&("cmd-{", Command::PrevTab)));
        let win = default_bindings(Platform::Windows);
        assert!(win.contains(&("ctrl-}", Command::NextTab)));
        assert!(win.contains(&("ctrl-{", Command::PrevTab)));
    }

    #[test]
    fn windows_never_binds_bare_ctrl_t_or_d() {
        let keys: Vec<&str> = default_bindings(Platform::Windows).iter().map(|(k, _)| *k).collect();
        assert!(!keys.contains(&"ctrl-t"));
        assert!(!keys.contains(&"ctrl-d"));
    }

    #[test]
    fn nine_project_bindings_each() {
        for platform in BOTH {
            let projects: Vec<u8> = default_bindings(platform)
                .iter()
                .filter_map(|(_, c)| match c {
                    Command::ActivateProject(n) => Some(*n),
                    _ => None,
                })
                .collect();
            assert_eq!(projects, (1..=9).collect::<Vec<u8>>());
        }
    }

    /// Windows の Ctrl+V は Claude Code の画像の貼り付けに使うため、端末へそのまま送る。
    #[test]
    fn copy_paste_bindings() {
        let mac = default_bindings(Platform::MacOs);
        assert!(mac.contains(&("cmd-c", Command::Copy)));
        assert!(mac.contains(&("cmd-v", Command::Paste)));
        let win = default_bindings(Platform::Windows);
        assert!(win.contains(&("ctrl-shift-c", Command::Copy)));
        assert!(win.contains(&("ctrl-shift-v", Command::Paste)));
        assert!(!win.iter().any(|(k, _)| *k == "ctrl-v"));
    }

    #[test]
    fn close_keys_per_platform() {
        assert!(default_bindings(Platform::MacOs).contains(&("cmd-w", Command::Close)));
        let win = default_bindings(Platform::Windows);
        assert!(win.contains(&("ctrl-shift-w", Command::Close)));
        // ターミナルの Ctrl+W（1 語を消す）を奪わないよう、Ctrl+W はターミナル以外でだけ効かせる。
        assert!(!win.iter().any(|(k, _)| *k == "ctrl-w"));
        assert!(context_bindings(Platform::Windows).contains(&("ctrl-w", Command::Close, NOT_TERMINAL)));
    }

    #[test]
    fn tree_and_file_view_keys() {
        assert!(default_bindings(Platform::MacOs).contains(&("cmd-b", Command::ToggleTree)));
        assert!(default_bindings(Platform::Windows).contains(&("ctrl-shift-b", Command::ToggleTree)));
        assert!(context_bindings(Platform::Windows).contains(&("ctrl-b", Command::ToggleTree, NOT_TERMINAL)));
        let mac = context_bindings(Platform::MacOs);
        assert!(mac.contains(&("cmd-s", Command::Save, FILE_VIEW)));
        let win = context_bindings(Platform::Windows);
        assert!(win.contains(&("ctrl-s", Command::Save, FILE_VIEW)));
    }

    #[test]
    fn file_view_has_only_save_and_send() {
        // 計画5の右パネルの履歴のキー（戻る／進む）は無くした。ファイルの中で効くのは保存と「Claude に渡す」だけ。
        for platform in BOTH {
            for (key, command, context) in context_bindings(platform) {
                if context == FILE_VIEW {
                    assert!(matches!(command, Command::Save | Command::SendToClaude), "{key}");
                }
            }
        }
    }

    #[test]
    fn send_to_claude_key() {
        assert!(context_bindings(Platform::MacOs).contains(&("cmd-shift-l", Command::SendToClaude, FILE_VIEW)));
        assert!(context_bindings(Platform::Windows).contains(&("ctrl-shift-l", Command::SendToClaude, FILE_VIEW)));
    }

    #[test]
    fn palette_and_search_keys() {
        let mac = default_bindings(Platform::MacOs);
        assert!(mac.contains(&("cmd-k", Command::OpenPalette)));
        assert!(mac.contains(&("cmd-p", Command::GoToFile)));
        assert!(mac.contains(&("cmd-shift-f", Command::FindInFiles)));
        let win = default_bindings(Platform::Windows);
        assert!(win.contains(&("ctrl-shift-k", Command::OpenPalette)));
        assert!(win.contains(&("ctrl-shift-p", Command::GoToFile)));
        assert!(win.contains(&("ctrl-shift-f", Command::FindInFiles)));
        // ターミナルの Ctrl+K / Ctrl+P（行末まで消す、履歴）を奪わない（§10）。
        assert!(!win.iter().any(|(k, _)| *k == "ctrl-k" || *k == "ctrl-p"));
        let wctx = context_bindings(Platform::Windows);
        assert!(wctx.contains(&("ctrl-k", Command::OpenPalette, NOT_TERMINAL)));
        assert!(wctx.contains(&("ctrl-p", Command::GoToFile, NOT_TERMINAL)));
    }

    #[test]
    fn terminal_find_keys() {
        assert!(context_bindings(Platform::MacOs).contains(&("cmd-f", Command::FindInTerminal, TERMINAL)));
        let win = context_bindings(Platform::Windows);
        assert!(win.contains(&("ctrl-alt-f", Command::FindInTerminal, TERMINAL)));
        // Ctrl+F はシェル（1 文字進む、fish の補完の確定）に渡す。
        assert!(!win.iter().any(|(k, _, _)| *k == "ctrl-f"));
    }

    #[test]
    fn mac_find_in_files_beats_editor_replace() {
        assert!(context_bindings(Platform::MacOs).contains(&("cmd-shift-f", Command::FindInFiles, INPUT)));
    }

    #[test]
    fn no_duplicate_keystrokes() {
        for platform in BOTH {
            let bindings = default_bindings(platform);
            let unique: HashSet<&str> = bindings.iter().map(|(k, _)| *k).collect();
            assert_eq!(unique.len(), bindings.len());
        }
    }
}
