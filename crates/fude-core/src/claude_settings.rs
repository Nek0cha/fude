//! Claude Code の settings.json に fude の hook を登録・解除する（設計書 `2026-11-15-settings-panel-hook-design.md` §5.5、§5.6）。
//! fude の hook 以外の中身とキーの並びは残す。

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::claude::{HookCommand, hook_config};

/// 登録先の settings.json の状態。`fude hook --status` はこれを 1 行の JSON で書く。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum HookStatus {
    /// fude の hook が、今の fude を指し、全部のイベントにそろっている。
    Registered,
    /// fude の hook が 1 つも無い（ファイルが無いときも）。
    NotRegistered,
    /// fude の hook はあるが、場所が違うかイベントが足りない。
    Outdated,
    /// 読めない（理由）。
    Unreadable(String),
}

/// パスの最後の部分（`/` と `\` で区切る）。
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// handler（`hooks[].hooks[]` の 1 つ）が fude の hook か（設計書 §5.5）。
pub fn is_fude_handler(handler: &Value) -> bool {
    let Some(command) = handler.get("command").and_then(Value::as_str) else { return false };
    match handler.get("args") {
        Some(args) => {
            let first = args.as_array().and_then(|a| a.first()).and_then(Value::as_str);
            first == Some("hook") && matches!(file_name(command), "fude" | "fude.exe")
        }
        None => {
            let words: Vec<&str> = command.split_whitespace().collect();
            words.len() >= 2 && words[words.len() - 1] == "hook" && file_name(words[words.len() - 2]) == "fude"
        }
    }
}

/// 読んだ文面。BOM と CRLF を覚えておき、書き出しで戻す。
struct Parsed {
    root: Map<String, Value>,
    bom: bool,
    crlf: bool,
}

/// text を読む。None・空白だけなら空のオブジェクト。一番外がオブジェクトでない、`hooks` がオブジェクトでないなら Err。
fn read(text: Option<&str>) -> Result<Parsed, String> {
    let text = text.unwrap_or("");
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let crlf = body.contains("\r\n");
    if body.trim().is_empty() {
        return Ok(Parsed { root: Map::new(), bom, crlf });
    }
    let value: Value = serde_json::from_str(body).map_err(|e| format!("JSON として読めません: {e}"))?;
    let Value::Object(root) = value else { return Err("一番外が { } ではありません".to_string()) };
    if root.get("hooks").is_some_and(|h| !h.is_object()) {
        return Err("\"hooks\" が { } ではありません".to_string());
    }
    Ok(Parsed { root, bom, crlf })
}

fn write(p: &Parsed) -> String {
    let mut out = serde_json::to_string_pretty(&Value::Object(p.root.clone())).unwrap_or_default();
    out.push('\n');
    if p.crlf {
        out = out.replace('\n', "\r\n");
    }
    if p.bom {
        out.insert(0, '\u{feff}');
    }
    out
}

/// hooks から fude の hook を取り除き、空になったまとまり・イベントを消す。`hooks` 自体は残す。
fn remove_fude(hooks: &mut Map<String, Value>) {
    for groups in hooks.values_mut() {
        let Some(list) = groups.as_array_mut() else { continue };
        for group in list.iter_mut() {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                handlers.retain(|h| !is_fude_handler(h));
            }
        }
        list.retain(|g| g.get("hooks").and_then(Value::as_array).is_none_or(|h| !h.is_empty()));
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|l| !l.is_empty()));
}

/// cmd の hook を登録した文面。`Ok(None)` は変わらない（書かない）。
pub fn install(text: Option<&str>, cmd: &HookCommand) -> Result<Option<String>, String> {
    let mut p = read(text)?;
    let before = p.root.clone();
    let hooks = p.root.entry("hooks").or_insert_with(|| Value::Object(Map::new())).as_object_mut().expect("read で確かめた");
    remove_fude(hooks);
    let config = hook_config(cmd);
    for (event, groups) in config["hooks"].as_object().expect("hook_config の形") {
        let list = hooks.entry(event.clone()).or_insert_with(|| Value::Array(Vec::new()));
        let Some(list) = list.as_array_mut() else { continue };
        for group in groups.as_array().expect("hook_config の形") {
            let matcher = group.get("matcher");
            let new_handlers = group["hooks"].as_array().expect("hook_config の形");
            let same = list.iter_mut().find(|g| g.get("matcher") == matcher && g.get("hooks").is_some_and(Value::is_array));
            match same {
                Some(g) => g["hooks"].as_array_mut().expect("確かめた").extend(new_handlers.iter().cloned()),
                None => list.push(group.clone()),
            }
        }
    }
    Ok((p.root != before || text.is_none_or(|t| t.trim().is_empty())).then(|| write(&p)))
}

/// fude の hook を外した文面。`Ok(None)` は変わらない（書かない）。
pub fn uninstall(text: Option<&str>) -> Result<Option<String>, String> {
    if text.is_none() {
        return Ok(None);
    }
    let mut p = read(text)?;
    let before = p.root.clone();
    if let Some(hooks) = p.root.get_mut("hooks").and_then(Value::as_object_mut) {
        remove_fude(hooks);
        if hooks.is_empty() {
            p.root.remove("hooks");
        }
    }
    Ok((p.root != before).then(|| write(&p)))
}

/// 登録先の状態（設計書 §5.4）。
pub fn status(text: Option<&str>, cmd: &HookCommand) -> HookStatus {
    let p = match read(text) {
        Ok(p) => p,
        Err(e) => return HookStatus::Unreadable(e),
    };
    let empty = Map::new();
    let hooks = p.root.get("hooks").and_then(Value::as_object).unwrap_or(&empty);
    let has_fude = hooks
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|g| g.get("hooks").and_then(Value::as_array))
        .flatten()
        .any(is_fude_handler);
    if !has_fude {
        return HookStatus::NotRegistered;
    }
    let config = hook_config(cmd);
    let complete = config["hooks"].as_object().expect("hook_config の形").iter().all(|(event, groups)| {
        groups.as_array().expect("hook_config の形").iter().all(|want| {
            let handler = &want["hooks"][0];
            hooks.get(event).and_then(Value::as_array).is_some_and(|list| {
                list.iter().any(|g| {
                    g.get("matcher") == want.get("matcher")
                        && g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
                            hs.iter().any(|h| h.get("command") == handler.get("command") && h.get("args") == handler.get("args"))
                        })
                })
            })
        })
    });
    if complete { HookStatus::Registered } else { HookStatus::Outdated }
}

/// ユーザー全体の settings.json の場所。`CLAUDE_CONFIG_DIR` があればその下、無ければ `<home>/.claude/`。
pub fn settings_path(env: &dyn Fn(&str) -> Option<String>, home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = env("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("settings.json"));
    }
    home.map(|h| h.join(".claude").join("settings.json"))
}

/// `fude hook --install / --uninstall / --status`（fude と fude-wsl で同じものを使う）。
/// command は登録する hook の command の上書き（`--command <s>`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookAdmin {
    Install { command: Option<String> },
    Uninstall,
    Status { command: Option<String> },
}

/// `hook` の後ろの引数。最初が `--install`、`--uninstall`、`--status` のどれでもなければ None。
pub fn parse_admin(args: &[String]) -> Option<Result<HookAdmin, String>> {
    const USAGE: &str = "使い方: fude hook --install [--command <コマンド>] | --uninstall | --status [--command <コマンド>]";
    let (first, rest) = args.split_first()?;
    let command = || match rest {
        [] => Ok(None),
        [flag, c] if flag == "--command" => Ok(Some(c.clone())),
        _ => Err(USAGE.to_string()),
    };
    Some(match first.as_str() {
        "--install" => command().map(|command| HookAdmin::Install { command }),
        "--status" => command().map(|command| HookAdmin::Status { command }),
        "--uninstall" if rest.is_empty() => Ok(HookAdmin::Uninstall),
        "--uninstall" => Err(USAGE.to_string()),
        _ => return None,
    })
}

/// ホームのフォルダ（HOME、無ければ USERPROFILE）。
pub fn home_from_env(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    env("HOME").or_else(|| env("USERPROFILE")).filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// admin を実行し、そのあとの状態を返す。make_cmd は `--command` の値（無ければ None）から HookCommand を作る。
pub fn run_admin(admin: &HookAdmin, path: Option<&Path>, make_cmd: &dyn Fn(Option<String>) -> HookCommand) -> HookStatus {
    let Some(path) = path else { return HookStatus::Unreadable("ホームのフォルダがわかりません".to_string()) };
    let text = match std::fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(t) => Some(t),
            Err(_) => return HookStatus::Unreadable("UTF-8 として読めません".to_string()),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return HookStatus::Unreadable(format!("読めません: {e}")),
    };
    let (cmd, edited) = match admin {
        HookAdmin::Status { command } => return status(text.as_deref(), &make_cmd(command.clone())),
        HookAdmin::Install { command } => {
            let cmd = make_cmd(command.clone());
            let edited = install(text.as_deref(), &cmd);
            (cmd, edited)
        }
        HookAdmin::Uninstall => (make_cmd(None), uninstall(text.as_deref())),
    };
    match edited {
        Err(e) => HookStatus::Unreadable(e),
        Ok(None) => status(text.as_deref(), &cmd),
        Ok(Some(new_text)) => match write_with_backup(path, &new_text) {
            Ok(()) => status(Some(&new_text), &cmd),
            Err(e) => HookStatus::Unreadable(format!("書けません: {e}")),
        },
    }
}

/// 元があれば `<名前>.bak` に写してから、同じフォルダの一時ファイルに書いて置き換える。フォルダが無ければ作る。
pub fn write_with_backup(path: &Path, new_text: &str) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| io::Error::other("settings.json のフォルダが無い"))?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().ok_or_else(|| io::Error::other("settings.json の名前が無い"))?.to_string_lossy().into_owned();
    if path.exists() {
        std::fs::copy(path, dir.join(format!("{name}.bak")))?;
    }
    // シンボリックリンク（dotfiles の管理など）なら、リンクを残してリンク先を置き換える。
    let is_link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let target = if is_link { std::fs::canonicalize(path)? } else { path.to_path_buf() };
    let target_dir = target.parent().ok_or_else(|| io::Error::other("settings.json のフォルダが無い"))?;
    let tmp = target_dir.join(format!("{name}.tmp.{}", std::process::id()));
    std::fs::write(&tmp, new_text)?;
    // 元の権限（0600 など）を引き継ぐ。
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::{HookCommand, hook_config};
    use serde_json::{Value, json};

    fn exec() -> HookCommand {
        HookCommand::Exec("C:/f/fude.exe".into())
    }

    fn parsed(text: &str) -> Value {
        serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap()
    }

    /// 開発者の手元と同じ形の settings.json（全イベントに古いパスの fude）。
    fn dev_settings() -> String {
        let mut v = hook_config(&HookCommand::Exec("C:/w/fude/target/debug/fude.exe".into()));
        let hooks = v["hooks"].as_object_mut().unwrap();
        hooks.insert("PreToolUse".into(), json!([{"matcher": "Bash", "hooks": [{"type": "command", "command": "mine.sh"}]}]));
        let mut root = serde_json::Map::new();
        root.insert("model".into(), json!("opus"));
        root.insert("hooks".into(), Value::Object(hooks.clone()));
        root.insert("theme".into(), json!("dark"));
        serde_json::to_string_pretty(&Value::Object(root)).unwrap()
    }

    #[test]
    fn exec_and_shell_fude_are_detected() {
        assert!(is_fude_handler(&json!({"type": "command", "command": "C:/x/target/debug/fude.exe", "args": ["hook"]})));
        assert!(is_fude_handler(&json!({"command": "C:\\x\\fude.exe", "args": ["hook"]})));
        assert!(is_fude_handler(&json!({"command": "/usr/local/bin/fude", "args": ["hook", "--x"]})));
        assert!(is_fude_handler(&json!({"command": "~/.local/bin/fude hook"})));
    }

    #[test]
    fn foreign_tools_are_not_fude() {
        assert!(!is_fude_handler(&json!({"command": "fude-lint", "args": ["hook"]})));
        assert!(!is_fude_handler(&json!({"command": "/opt/fudepack/bin/tool hook"})));
        assert!(!is_fude_handler(&json!({"command": "C:/x/fude.exe", "args": ["open"]})));
        assert!(!is_fude_handler(&json!({"command": "echo fude"})));
        assert!(!is_fude_handler(&json!({"command": "fude hook", "args": ["x"]})));
    }

    #[test]
    fn install_into_missing_file() {
        let out = install(None, &exec()).unwrap().unwrap();
        assert_eq!(parsed(&out), hook_config(&exec()));
        assert!(out.ends_with("}\n"));
    }

    #[test]
    fn install_replaces_old_fude_and_keeps_others() {
        let out = install(Some(&dev_settings()), &exec()).unwrap().unwrap();
        let v = parsed(&out);
        assert_eq!(v["model"], "opus");
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "mine.sh");
        assert_eq!(v["hooks"]["Stop"], hook_config(&exec())["hooks"]["Stop"]);
        assert_eq!(v["hooks"]["PostToolUse"], hook_config(&exec())["hooks"]["PostToolUse"]);
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["model", "hooks", "theme"]);
        assert!(!out.contains("target/debug"));
    }

    #[test]
    fn install_keeps_foreign_handler_in_same_group() {
        let text = r#"{"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": "notify.sh"},
            {"type": "command", "command": "C:/old/fude.exe", "args": ["hook"]}
        ]}]}}"#;
        let v = parsed(&install(Some(text), &exec()).unwrap().unwrap());
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        let handlers = stop[0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.len(), 2);
        assert_eq!(handlers[0]["command"], "notify.sh");
        assert_eq!(handlers[1]["command"], "C:/f/fude.exe");
    }

    #[test]
    fn install_twice_is_unchanged() {
        let once = install(None, &exec()).unwrap().unwrap();
        assert_eq!(install(Some(&once), &exec()), Ok(None));
    }

    #[test]
    fn install_preserves_crlf_and_bom() {
        let text = "\u{feff}{\r\n  \"model\": \"opus\"\r\n}\r\n";
        let out = install(Some(text), &exec()).unwrap().unwrap();
        assert!(out.starts_with('\u{feff}'));
        assert!(out.contains("\r\n"));
        assert!(!out.replace("\r\n", "").contains('\n'));
        assert_eq!(parsed(&out)["model"], "opus");
    }

    #[test]
    fn install_rejects_broken_json() {
        assert!(install(Some("{ \"hooks\": "), &exec()).is_err());
        assert!(install(Some("[]"), &exec()).is_err());
        assert!(install(Some(r#"{"hooks": []}"#), &exec()).is_err());
    }

    #[test]
    fn empty_text_is_like_missing() {
        let out = install(Some("  \n"), &exec()).unwrap().unwrap();
        assert_eq!(parsed(&out), hook_config(&exec()));
    }

    #[test]
    fn uninstall_removes_only_fude_and_empty_parts() {
        let out = uninstall(Some(&dev_settings())).unwrap().unwrap();
        let v = parsed(&out);
        let hooks = v["hooks"].as_object().unwrap();
        assert_eq!(hooks.keys().collect::<Vec<_>>(), ["PreToolUse"]);
        assert_eq!(v["model"], "opus");

        let only_fude = install(None, &exec()).unwrap().unwrap();
        let v = parsed(&uninstall(Some(&only_fude)).unwrap().unwrap());
        assert_eq!(v, json!({}));

        assert_eq!(uninstall(None), Ok(None));
        assert_eq!(uninstall(Some(r#"{"model": "x"}"#)), Ok(None));
    }

    #[test]
    fn status_cases() {
        assert_eq!(status(None, &exec()), HookStatus::NotRegistered);
        assert_eq!(status(Some(r#"{"model": "x"}"#), &exec()), HookStatus::NotRegistered);
        let once = install(None, &exec()).unwrap().unwrap();
        assert_eq!(status(Some(&once), &exec()), HookStatus::Registered);
        assert_eq!(status(Some(&dev_settings()), &exec()), HookStatus::Outdated);
        let mut v = parsed(&once);
        v["hooks"].as_object_mut().unwrap().remove("PostToolUse");
        assert_eq!(status(Some(&v.to_string()), &exec()), HookStatus::Outdated);
        assert!(matches!(status(Some("{"), &exec()), HookStatus::Unreadable(_)));
    }

    #[test]
    fn status_serializes_as_one_line() {
        assert_eq!(serde_json::to_string(&HookStatus::Registered).unwrap(), r#"{"status":"registered"}"#);
        assert_eq!(serde_json::to_string(&HookStatus::Unreadable("x".into())).unwrap(), r#"{"status":"unreadable","reason":"x"}"#);
        let back: HookStatus = serde_json::from_str(r#"{"status":"not_registered"}"#).unwrap();
        assert_eq!(back, HookStatus::NotRegistered);
    }

    #[test]
    fn settings_path_prefers_claude_config_dir() {
        let home = Some(std::path::PathBuf::from("/h"));
        let env = |k: &str| (k == "CLAUDE_CONFIG_DIR").then(|| "/c".to_string());
        assert_eq!(settings_path(&env, home.clone()), Some(std::path::PathBuf::from("/c").join("settings.json")));
        assert_eq!(settings_path(&|_| None, home), Some(std::path::PathBuf::from("/h").join(".claude").join("settings.json")));
        assert_eq!(settings_path(&|_| Some(String::new()), None), None);
    }

    #[test]
    fn parses_admin_args() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_admin(&a(&["--install"])), Some(Ok(HookAdmin::Install { command: None })));
        assert_eq!(
            parse_admin(&a(&["--install", "--command", "C:/a/fude.exe"])),
            Some(Ok(HookAdmin::Install { command: Some("C:/a/fude.exe".into()) }))
        );
        assert_eq!(parse_admin(&a(&["--uninstall"])), Some(Ok(HookAdmin::Uninstall)));
        assert_eq!(parse_admin(&a(&["--status"])), Some(Ok(HookAdmin::Status { command: None })));
        assert!(matches!(parse_admin(&a(&["--install", "--status"])), Some(Err(_))));
        assert!(matches!(parse_admin(&a(&["--uninstall", "--command", "x"])), Some(Err(_))));
        assert_eq!(parse_admin(&a(&["--signal"])), None);
        assert_eq!(parse_admin(&a(&[])), None);
    }

    #[test]
    fn run_admin_install_status_uninstall() {
        let dir = std::env::temp_dir().join(format!("fude-claude-admin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let make = |c: Option<String>| HookCommand::Exec(c.unwrap_or_else(|| "C:/f/fude.exe".into()));
        assert_eq!(run_admin(&HookAdmin::Status { command: None }, Some(&path), &make), HookStatus::NotRegistered);
        assert_eq!(run_admin(&HookAdmin::Install { command: None }, Some(&path), &make), HookStatus::Registered);
        assert_eq!(run_admin(&HookAdmin::Status { command: Some("C:/g/fude.exe".into()) }, Some(&path), &make), HookStatus::Outdated);
        assert_eq!(run_admin(&HookAdmin::Uninstall, Some(&path), &make), HookStatus::NotRegistered);
        std::fs::write(&path, "{").unwrap();
        assert!(matches!(run_admin(&HookAdmin::Install { command: None }, Some(&path), &make), HookStatus::Unreadable(_)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{");
        assert!(matches!(run_admin(&HookAdmin::Status { command: None }, None, &make), HookStatus::Unreadable(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_with_backup_follows_symlink_and_keeps_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("fude-claude-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        std::fs::create_dir_all(dir.join("home")).unwrap();
        let real = dir.join("dotfiles").join("settings.json");
        std::fs::write(&real, "one").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("home").join("settings.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_with_backup(&link, "two").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "リンクのまま");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "two");
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(dir.join("home").join("settings.json.bak")).unwrap(), "one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_with_backup_keeps_previous() {
        let dir = std::env::temp_dir().join(format!("fude-claude-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("settings.json");
        write_with_backup(&path, "one").unwrap();
        assert!(!dir.join("sub").join("settings.json.bak").exists());
        write_with_backup(&path, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(std::fs::read_to_string(dir.join("sub").join("settings.json.bak")).unwrap(), "one");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
