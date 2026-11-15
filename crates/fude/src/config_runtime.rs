//! 設定ファイルの読み込み・生成・見張り・反映（設計書 `2026-11-15-config-theme-design.md` §5.3、§5.4）。
//! アプリ全体に効くもの（テーマ、フォント、キー）はここで反映し、ConfigState を差し替える。ウィンドウや
//! ターミナルに効くものは、WorkspaceView が ConfigState の変化を見て反映する。

use std::path::{Path, PathBuf};
use std::time::Duration;

use fude_core::settings::{ConfigIssue, DEFAULT_CONFIG, ParseOutcome, Settings, WindowSettings, parse};
use futures::StreamExt;
use gpui_kit::{App, Global, WindowBackgroundAppearance};

/// 今の設定と、設定ファイルの問題。
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigState {
    pub settings: Settings,
    pub issues: Vec<ConfigIssue>,
    pub path: Option<PathBuf>,
}

impl Global for ConfigState {}

/// 保存の出来事をまとめる時間。
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(150);

/// 設定ファイルを読む。無ければ既定の文面で作る。読めなければ既定値で動かし、ファイルは書き換えない。
pub fn load_or_create(path: Option<&Path>) -> ConfigState {
    let Some(path) = path else {
        return ConfigState { settings: Settings::default(), issues: Vec::new(), path: None };
    };
    let state = |settings, issues| ConfigState { settings, issues, path: Some(path.to_path_buf()) };
    match std::fs::read(path) {
        Ok(bytes) => match fude_core::settings::decode_config(&bytes).map(|text| parse(&text)) {
            Ok(ParseOutcome::Ok { settings, issues }) => state(settings, issues),
            Ok(ParseOutcome::Unreadable(issue)) | Err(issue) => state(Settings::default(), vec![issue]),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(path, DEFAULT_CONFIG));
            let issues = match written {
                Ok(()) => Vec::new(),
                Err(e) => vec![ConfigIssue { line: None, message: format!("設定ファイルを作れません（{}）: {e}", path.display()) }],
            };
            state(Settings::default(), issues)
        }
        Err(e) => state(Settings::default(), vec![ConfigIssue { line: None, message: format!("設定ファイルを読めません: {e}") }]),
    }
}

/// 読み直した文面から設定を作る。読めなければ（空、書きかけを含む）prev のまま。
pub fn reload(prev: &Settings, text: &str) -> (Settings, Vec<ConfigIssue>) {
    match parse(text) {
        ParseOutcome::Ok { settings, issues } => (settings, issues),
        ParseOutcome::Unreadable(issue) => (prev.clone(), vec![issue]),
    }
}

/// ウィンドウの背景の方式。不透明なら Opaque、ぼかすなら Blurred、それ以外は Transparent。
pub fn background_appearance(window: &WindowSettings) -> WindowBackgroundAppearance {
    if window.background_opacity >= 1.0 {
        WindowBackgroundAppearance::Opaque
    } else if window.blur {
        WindowBackgroundAppearance::Blurred
    } else {
        WindowBackgroundAppearance::Transparent
    }
}

/// 知らせに出す文。
pub fn issue_text(issue: &ConfigIssue) -> String {
    match issue.line {
        Some(line) => format!("設定ファイルの {line} 行目: {}", issue.message),
        None => format!("設定ファイル: {}", issue.message),
    }
}

/// アプリ全体に効く設定を反映する（起動時は old を None にする）。キーの問題を返す。
pub fn apply_global(cx: &mut App, old: Option<&Settings>, new: &Settings) -> Vec<ConfigIssue> {
    crate::theme::set_fonts(&new.font);
    cx.set_global(crate::themes::build_theme(new));
    crate::theme::apply_component_theme(cx);
    if old.is_some_and(|old| old.keybindings == new.keybindings) {
        return crate::workspace_view::valid_bindings(new).1;
    }
    crate::workspace_view::install_keys(cx, new)
}

/// 値が変わらなかったときの問題: 読み込みの問題と、キーの問題（設定から作り直すので重ならない）。
pub fn unchanged_issues(mut parse_issues: Vec<ConfigIssue>, settings: &Settings) -> Vec<ConfigIssue> {
    parse_issues.extend(crate::workspace_view::valid_bindings(settings).1);
    parse_issues
}

/// 見張りを捨てないために持つ。
struct ConfigWatcher(#[allow(dead_code)] notify::RecommendedWatcher);

impl Global for ConfigWatcher {}

/// 設定ファイルのあるフォルダを見張り、保存されたら読み直す。エディタの「一時ファイルに書いて rename する」
/// 保存も拾うため、ファイルではなくフォルダを見張る。
pub fn start_watching(cx: &mut App) {
    use notify::Watcher;
    let Some(path) = cx.global::<ConfigState>().path.clone() else { return };
    let (Some(dir), Some(name)) = (path.parent().map(Path::to_path_buf), path.file_name().map(|n| n.to_os_string())) else { return };
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res
            && ev.paths.iter().any(|p| p.file_name() == Some(name.as_os_str()))
        {
            let _ = tx.unbounded_send(());
        }
    });
    let mut watcher = match watcher {
        Ok(w) => w,
        Err(e) => {
            eprintln!("設定ファイルを見張れない: {e}");
            return;
        }
    };
    if let Err(e) = watcher.watch(&dir, notify::RecursiveMode::NonRecursive) {
        eprintln!("設定ファイルを見張れない: {e}");
        return;
    }
    cx.set_global(ConfigWatcher(watcher));
    cx.spawn(async move |cx| {
        while rx.next().await.is_some() {
            cx.background_executor().timer(RELOAD_DEBOUNCE).await;
            while rx.try_recv().is_ok() {}
            cx.update(reload_now);
        }
    })
    .detach();
}

/// 設定ファイルを読み直し、変わっていれば反映する。自分で書いた保存で二度呼ばれても、値が同じなら何もしない。
pub fn reload_now(cx: &mut App) {
    let state = cx.global::<ConfigState>().clone();
    let Some(path) = state.path.clone() else { return };
    let read = std::fs::read(&path).map_err(|e| ConfigIssue { line: None, message: format!("設定ファイルを読めません: {e}") });
    let (settings, mut issues) = match read.and_then(|b| fude_core::settings::decode_config(&b)) {
        Ok(text) => reload(&state.settings, &text),
        // 消えた・読めないときは今の値のまま。
        Err(issue) => (state.settings.clone(), vec![issue]),
    };
    if settings == state.settings {
        let issues = unchanged_issues(issues, &settings);
        if issues != state.issues {
            cx.set_global(ConfigState { issues, ..state });
            cx.refresh_windows();
        }
        return;
    }
    issues.extend(apply_global(cx, Some(&state.settings), &settings));
    cx.set_global(ConfigState { settings, issues, path: Some(path) });
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;
    use fude_core::settings::{DEFAULT_CONFIG, Settings, ThemeId, WindowSettings};

    #[test]
    fn missing_file_is_created_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fude").join("config.toml");
        let state = load_or_create(Some(&path));
        assert_eq!(state.settings, Settings::default());
        assert!(state.issues.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
    }

    #[test]
    fn existing_file_is_read_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "theme = \"oled\"\n").unwrap();
        let state = load_or_create(Some(&path));
        assert_eq!(state.settings.theme, ThemeId::Oled);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theme = \"oled\"\n");
    }

    #[test]
    fn unreadable_file_at_start_uses_defaults_and_keeps_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "theme = ").unwrap();
        let state = load_or_create(Some(&path));
        assert_eq!(state.settings, Settings::default());
        assert_eq!(state.issues.len(), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theme = ");
    }

    #[test]
    fn unreadable_reload_keeps_previous() {
        let prev = Settings { theme: ThemeId::Dracula, ..Settings::default() };
        let (settings, issues) = reload(&prev, "");
        assert_eq!(settings.theme, ThemeId::Dracula);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn partial_write_keeps_previous() {
        let prev = Settings { theme: ThemeId::Dracula, ..Settings::default() };
        let (settings, _) = reload(&prev, "theme = \"to");
        assert_eq!(settings, prev);
    }

    #[test]
    fn readable_reload_replaces() {
        let prev = Settings { theme: ThemeId::Dracula, ..Settings::default() };
        let (settings, issues) = reload(&prev, "theme = \"oled\"\n");
        assert_eq!(settings.theme, ThemeId::Oled);
        assert!(issues.is_empty());
    }

    #[test]
    fn background_appearance_choice() {
        use gpui_kit::WindowBackgroundAppearance as A;
        assert_eq!(background_appearance(&WindowSettings { background_opacity: 1.0, blur: true }), A::Opaque);
        assert_eq!(background_appearance(&WindowSettings { background_opacity: 0.62, blur: true }), A::Blurred);
        assert_eq!(background_appearance(&WindowSettings { background_opacity: 0.62, blur: false }), A::Transparent);
    }

    #[test]
    fn unwritable_dir_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        // ファイルの下にフォルダは作れない。
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "").unwrap();
        let state = load_or_create(Some(&blocker.join("fude").join("config.toml")));
        assert_eq!(state.settings, Settings::default());
        assert_eq!(state.issues.len(), 1);
    }

    #[test]
    fn no_path_means_defaults_silently() {
        let state = load_or_create(None);
        assert_eq!(state.settings, Settings::default());
        assert!(state.issues.is_empty());
    }

    #[test]
    fn issue_text_has_line() {
        use fude_core::settings::ConfigIssue;
        assert_eq!(issue_text(&ConfigIssue { line: Some(3), message: "x は y".into() }), "設定ファイルの 3 行目: x は y");
        assert_eq!(issue_text(&ConfigIssue { line: None, message: "x は y".into() }), "設定ファイル: x は y");
    }
}
