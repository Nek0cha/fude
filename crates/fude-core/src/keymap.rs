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
}

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
            ],
            PROJECT_KEYS_WIN,
        ),
    };
    let projects = projects.into_iter().zip(1u8..).map(|(k, n)| (k, Command::ActivateProject(n)));
    fixed.into_iter().chain(projects).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const BOTH: [Platform; 2] = [Platform::MacOs, Platform::Windows];

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
    fn no_duplicate_keystrokes() {
        for platform in BOTH {
            let bindings = default_bindings(platform);
            let unique: HashSet<&str> = bindings.iter().map(|(k, _)| *k).collect();
            assert_eq!(unique.len(), bindings.len());
        }
    }
}
