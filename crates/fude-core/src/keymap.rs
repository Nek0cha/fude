//! プラットフォームごとのショートカットの割り当て表（設計書 §10）と、設定ファイルによる上書き。

use crate::settings::{ConfigIssue, KeybindingOverrides};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// Markdown の表示モードとソースモードの切り替え。ファイルの中だけで効く。
    ToggleMarkdownMode,
    /// 設定パネルを開く（9b）。
    OpenSettings,
}

impl Command {
    /// 設定ファイルに書く名前（snake_case）。
    pub fn name(self) -> String {
        let fixed = match self {
            Command::NewTab => "new_tab",
            Command::SplitTab => "split_tab",
            Command::NextTab => "next_tab",
            Command::PrevTab => "prev_tab",
            Command::ActivateProject(n) => return format!("activate_project_{n}"),
            Command::Copy => "copy",
            Command::Paste => "paste",
            Command::Close => "close",
            Command::ToggleTree => "toggle_tree",
            Command::Save => "save",
            Command::OpenPalette => "open_palette",
            Command::GoToFile => "go_to_file",
            Command::FindInFiles => "find_in_files",
            Command::FindInTerminal => "find_in_terminal",
            Command::ToggleRouteMap => "toggle_route_map",
            Command::SendToClaude => "send_to_claude",
            Command::ToggleMarkdownMode => "toggle_markdown_mode",
            Command::OpenSettings => "open_settings",
        };
        fixed.to_string()
    }

    pub fn from_name(name: &str) -> Option<Command> {
        if let Some(n) = name.strip_prefix("activate_project_") {
            return n.parse::<u8>().ok().filter(|n| (1..=9).contains(n) && name.len() == "activate_project_".len() + 1).map(Command::ActivateProject);
        }
        const FIXED: [Command; 17] = [
            Command::NewTab,
            Command::SplitTab,
            Command::NextTab,
            Command::PrevTab,
            Command::Copy,
            Command::Paste,
            Command::Close,
            Command::ToggleTree,
            Command::Save,
            Command::OpenPalette,
            Command::GoToFile,
            Command::FindInFiles,
            Command::FindInTerminal,
            Command::ToggleRouteMap,
            Command::SendToClaude,
            Command::ToggleMarkdownMode,
            Command::OpenSettings,
        ];
        FIXED.into_iter().find(|c| c.name() == name)
    }
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
                ("cmd-,", Command::OpenSettings),
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
                // Ctrl+\ はシェルの SIGQUIT と重なるため、どこでも効くのは Shift 付きだけ（§10）。
                ("ctrl-|", Command::ToggleRouteMap),
                // Ctrl+, はシェルのキー操作と重ならないので、ターミナルの中でも効かせる（Windows Terminal と同じ）。
                ("ctrl-,", Command::OpenSettings),
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
            ("cmd-e", Command::ToggleMarkdownMode, FILE_VIEW),
            ("cmd-f", Command::FindInTerminal, TERMINAL),
            // gpui-kit の入力欄は ⌘⇧F を置換に割り当てているため、全体検索を優先させる（置換は ⌘⌥F）。
            ("cmd-shift-f", Command::FindInFiles, INPUT),
        ],
        Platform::Windows => vec![
            ("ctrl-w", Command::Close, NOT_TERMINAL),
            ("ctrl-b", Command::ToggleTree, NOT_TERMINAL),
            ("ctrl-s", Command::Save, FILE_VIEW),
            ("ctrl-shift-l", Command::SendToClaude, FILE_VIEW),
            // Ctrl+E はシェル（行末へ）に渡すため、ファイルの中だけで効かせる。
            ("ctrl-e", Command::ToggleMarkdownMode, FILE_VIEW),
            ("ctrl-k", Command::OpenPalette, NOT_TERMINAL),
            ("ctrl-p", Command::GoToFile, NOT_TERMINAL),
            ("ctrl-\\", Command::ToggleRouteMap, NOT_TERMINAL),
            // Ctrl+F はシェル（1 文字進む、fish の補完の確定）に、Ctrl+Shift+F は全体検索に使う。
            ("ctrl-alt-f", Command::FindInTerminal, TERMINAL),
        ],
    }
}

/// 登録するキー 1 つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub key: String,
    pub command: Command,
    pub context: Option<&'static str>,
}

/// 既定の割り当て表に、設定ファイルの今の OS の表の上書きを当てたもの。
/// 書いた Command は既定の行をすべて消し、書いたキーで置き換える（[] なら外すだけ）。新しいキーの文脈は、その Command が
/// default_bindings にあれば None（Copy と Paste は Terminal）、無ければ context_bindings の最初の行の文脈。
/// 知らない Command の名前は捨てて issue を積む。キーの表記の検査は呼び出し側（GPUI）が行う。
pub fn bindings(platform: Platform, overrides: &KeybindingOverrides) -> (Vec<Binding>, Vec<ConfigIssue>) {
    let global_context = |c: Command| matches!(c, Command::Copy | Command::Paste).then_some(TERMINAL);
    let defaults = default_bindings(platform);
    let contexts = context_bindings(platform);
    let mut list: Vec<Binding> = defaults
        .iter()
        .map(|&(k, c)| Binding { key: k.to_string(), command: c, context: global_context(c) })
        .chain(contexts.iter().map(|&(k, c, ctx)| Binding { key: k.to_string(), command: c, context: Some(ctx) }))
        .collect();
    let table = match platform {
        Platform::Windows => (&overrides.windows, "windows"),
        Platform::MacOs => (&overrides.macos, "macos"),
    };
    let mut issues = Vec::new();
    for (name, keys) in table.0 {
        let Some(command) = Command::from_name(name) else {
            let line = overrides.lines.get(&format!("{}.{name}", table.1)).copied();
            issues.push(ConfigIssue { line, message: format!("keybindings.{}.{name} は知らない Command の名前です", table.1) });
            continue;
        };
        let context = if defaults.iter().any(|&(_, c)| c == command) {
            global_context(command)
        } else {
            contexts.iter().find(|&&(_, c, _)| c == command).map(|&(_, _, ctx)| ctx)
        };
        // Windows で既定に「ターミナル以外」の行がある Command は、Shift も Alt も付かない Ctrl キーを既定と同じく
        // ターミナルの外だけで効かせる（Ctrl+K、Ctrl+W などのシェルのキー操作を奪わないため）。
        let has_not_terminal = contexts.iter().any(|&(_, c, ctx)| c == command && ctx == NOT_TERMINAL);
        let context_for = |key: &str| {
            let shell_key = key.starts_with("ctrl-") && !key.contains("shift-") && !key.contains("alt-");
            if platform == Platform::Windows && has_not_terminal && shell_key { Some(NOT_TERMINAL) } else { context }
        };
        list.retain(|b| b.command != command);
        list.extend(keys.iter().map(|k| Binding { key: k.clone(), command, context: context_for(k) }));
    }
    (list, issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const BOTH: [Platform; 2] = [Platform::MacOs, Platform::Windows];

    #[test]
    fn route_map_key() {
        assert!(default_bindings(Platform::MacOs).contains(&("cmd-\\", Command::ToggleRouteMap)));
        // Windows の Ctrl+\ はターミナルの SIGQUIT（0x1C）に渡す。どこでも効くのは Shift 付き（GPUI は `|` で受け取る）。
        let win = default_bindings(Platform::Windows);
        assert!(win.contains(&("ctrl-|", Command::ToggleRouteMap)));
        assert!(!win.iter().any(|(k, _)| *k == "ctrl-\\"));
        assert!(context_bindings(Platform::Windows).contains(&("ctrl-\\", Command::ToggleRouteMap, NOT_TERMINAL)));
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
    fn file_view_has_only_save_send_and_markdown() {
        // 計画5の右パネルの履歴のキー（戻る／進む）は無くした。ファイルの中で効くのは保存、「Claude に渡す」、表示／ソースの切り替えだけ。
        for platform in BOTH {
            for (key, command, context) in context_bindings(platform) {
                if context == FILE_VIEW {
                    assert!(matches!(command, Command::Save | Command::SendToClaude | Command::ToggleMarkdownMode), "{key}");
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

    #[test]
    fn markdown_toggle_is_file_view_only() {
        assert!(context_bindings(Platform::MacOs).contains(&("cmd-e", Command::ToggleMarkdownMode, FILE_VIEW)));
        assert!(context_bindings(Platform::Windows).contains(&("ctrl-e", Command::ToggleMarkdownMode, FILE_VIEW)));
        assert!(!default_bindings(Platform::Windows).iter().any(|(k, _)| *k == "ctrl-e"));
    }

    fn overrides(windows: &[(&str, &[&str])], macos: &[(&str, &[&str])]) -> KeybindingOverrides {
        let conv = |rows: &[(&str, &[&str])]| rows.iter().map(|(c, keys)| (c.to_string(), keys.iter().map(|k| k.to_string()).collect())).collect();
        KeybindingOverrides { windows: conv(windows), macos: conv(macos), lines: Default::default() }
    }

    fn rows_of(list: &[Binding], command: Command) -> Vec<(String, Option<&'static str>)> {
        list.iter().filter(|b| b.command == command).map(|b| (b.key.clone(), b.context)).collect()
    }

    #[test]
    fn open_settings_keys() {
        let mac = default_bindings(Platform::MacOs);
        assert!(mac.contains(&("cmd-,", Command::OpenSettings)));
        let win = default_bindings(Platform::Windows);
        assert!(win.contains(&("ctrl-,", Command::OpenSettings)));
        let (list, _) = bindings(Platform::Windows, &KeybindingOverrides::default());
        let row = list.iter().find(|b| b.command == Command::OpenSettings).unwrap();
        assert_eq!(row.context, None, "ターミナルの中でも開ける");
        assert_eq!(Command::from_name("open_settings"), Some(Command::OpenSettings));
        assert_eq!(Command::OpenSettings.name(), "open_settings");
    }

    #[test]
    fn command_names_round_trip() {
        for platform in BOTH {
            let commands = default_bindings(platform).into_iter().map(|(_, c)| c).chain(context_bindings(platform).into_iter().map(|(_, c, _)| c));
            for c in commands {
                assert_eq!(Command::from_name(&c.name()), Some(c), "{c:?}");
            }
        }
        assert_eq!(Command::ActivateProject(3).name(), "activate_project_3");
        assert_eq!(Command::from_name("activate_project_0"), None);
        assert_eq!(Command::from_name("activate_project_10"), None);
    }

    #[test]
    fn no_overrides_equals_defaults() {
        for platform in BOTH {
            let (list, issues) = bindings(platform, &KeybindingOverrides::default());
            assert!(issues.is_empty());
            let got: HashSet<(String, Command, Option<&str>)> = list.into_iter().map(|b| (b.key, b.command, b.context)).collect();
            let mut want: HashSet<(String, Command, Option<&str>)> = default_bindings(platform)
                .into_iter()
                .map(|(k, c)| (k.to_string(), c, matches!(c, Command::Copy | Command::Paste).then_some(TERMINAL)))
                .collect();
            want.extend(context_bindings(platform).into_iter().map(|(k, c, ctx)| (k.to_string(), c, Some(ctx))));
            assert_eq!(got, want);
        }
    }

    #[test]
    fn override_replaces_all_rows_of_command() {
        let (list, _) = bindings(Platform::Windows, &overrides(&[("close", &["ctrl-q"])], &[]));
        assert_eq!(rows_of(&list, Command::Close), vec![("ctrl-q".to_string(), Some(NOT_TERMINAL))]);
    }

    #[test]
    fn context_only_command_keeps_its_context() {
        let (list, _) = bindings(Platform::Windows, &overrides(&[("save", &["ctrl-alt-s"])], &[]));
        assert_eq!(rows_of(&list, Command::Save), vec![("ctrl-alt-s".to_string(), Some(FILE_VIEW))]);
    }

    #[test]
    fn copy_keeps_terminal_context() {
        let (list, _) = bindings(Platform::MacOs, &overrides(&[], &[("copy", &["cmd-shift-c"])]));
        assert_eq!(rows_of(&list, Command::Copy), vec![("cmd-shift-c".to_string(), Some(TERMINAL))]);
    }

    /// Windows でシェルと重なる Shift なしの Ctrl キーは、既定と同じく「ターミナル以外」でだけ効かせる。
    #[test]
    fn shell_keys_stay_out_of_terminal() {
        let (list, _) = bindings(Platform::Windows, &overrides(&[("open_palette", &["ctrl-shift-k", "ctrl-k"]), ("close", &["ctrl-w"])], &[]));
        assert_eq!(
            rows_of(&list, Command::OpenPalette),
            vec![("ctrl-shift-k".to_string(), None), ("ctrl-k".to_string(), Some(NOT_TERMINAL))]
        );
        assert_eq!(rows_of(&list, Command::Close), vec![("ctrl-w".to_string(), Some(NOT_TERMINAL))]);
        // 「ターミナル以外」の行を持たない Command はそのまま。
        let (list, _) = bindings(Platform::Windows, &overrides(&[("new_tab", &["ctrl-n"])], &[]));
        assert_eq!(rows_of(&list, Command::NewTab), vec![("ctrl-n".to_string(), None)]);
        // macOS は ⌘ なのでシェルと重ならない。
        let (list, _) = bindings(Platform::MacOs, &overrides(&[], &[("open_palette", &["ctrl-k"])]));
        assert_eq!(rows_of(&list, Command::OpenPalette), vec![("ctrl-k".to_string(), None)]);
    }

    #[test]
    fn empty_list_unbinds() {
        let (list, issues) = bindings(Platform::Windows, &overrides(&[("toggle_tree", &[])], &[]));
        assert!(issues.is_empty());
        assert!(rows_of(&list, Command::ToggleTree).is_empty());
    }

    #[test]
    fn other_platform_table_is_ignored() {
        let (list, _) = bindings(Platform::Windows, &overrides(&[], &[("close", &[])]));
        assert_eq!(rows_of(&list, Command::Close).len(), 2);
    }

    #[test]
    fn unknown_command_issue_has_line() {
        let mut o = overrides(&[("nope", &["ctrl-1"])], &[]);
        o.lines.insert("windows.nope".into(), 7);
        let (_, issues) = bindings(Platform::Windows, &o);
        assert_eq!(issues[0].line, Some(7));
    }

    #[test]
    fn unknown_command_is_an_issue() {
        let (list, issues) = bindings(Platform::Windows, &overrides(&[("nope", &["ctrl-1"])], &[]));
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("nope"));
        assert_eq!(list.len(), default_bindings(Platform::Windows).len() + context_bindings(Platform::Windows).len());
    }
}
