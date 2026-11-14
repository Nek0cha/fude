//! Claude Code との連携（設計書 §5.8）。hook の入力を小さな信号にし、ペインごとの返事待ちとタスクを持つ。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub subject: String,
    pub status: TodoStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TodoChange {
    Create { id: String, subject: String },
    /// status が None なら件名だけ変える。deleted は Remove にする。
    Update { id: String, status: Option<TodoStatus>, subject: Option<String> },
    Remove { id: String },
    /// TodoWrite（一覧ごと置き換える。id は 1 始まりの番号の文字列）。
    Replace { items: Vec<TodoItem> },
}

/// hook から fude に送る小さな信号。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HookSignal {
    /// 入力を受けて作業中。
    Working,
    /// 返事や許可を待っている（返事を終えたときも含む）。
    Waiting,
    /// セッションが始まった。reset なら（新しく始めた、/clear）タスクも消す。
    SessionStart { reset: bool },
    SessionEnd,
    Todo { change: TodoChange },
}

/// タスクの件名の上限（文字数）。
pub const SUBJECT_MAX_CHARS: usize = 200;

/// 返事待ちにする通知の種類。
const WAITING_NOTIFICATIONS: [&str; 5] =
    ["permission_prompt", "idle_prompt", "elicitation_dialog", "elicitation_url_dialog", "agent_needs_input"];

fn subject(v: &Value) -> Option<String> {
    v.as_str().map(|s| s.chars().take(SUBJECT_MAX_CHARS).collect())
}

/// 文字列か数値の id を文字列にする。
fn id_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn status_of(v: &Value) -> Option<Option<TodoStatus>> {
    match v.as_str()? {
        "pending" => Some(Some(TodoStatus::Pending)),
        "in_progress" => Some(Some(TodoStatus::InProgress)),
        "completed" => Some(Some(TodoStatus::Completed)),
        "deleted" => Some(None),
        _ => None,
    }
}

fn todo_change(input: &Value) -> Option<TodoChange> {
    let tool_input = &input["tool_input"];
    match input["tool_name"].as_str()? {
        "TaskCreate" => {
            let task = &input["tool_response"]["task"];
            let id = id_of(&task["id"])?;
            let subject = subject(&tool_input["subject"]).or_else(|| subject(&task["subject"]))?;
            Some(TodoChange::Create { id, subject })
        }
        "TaskUpdate" => {
            let id = id_of(&tool_input["taskId"])?;
            let status = match &tool_input["status"] {
                Value::Null => None,
                v => match status_of(v)? {
                    Some(s) => Some(s),
                    None => return Some(TodoChange::Remove { id }),
                },
            };
            let subject = subject(&tool_input["subject"]);
            if status.is_none() && subject.is_none() {
                return None;
            }
            Some(TodoChange::Update { id, status, subject })
        }
        "TodoWrite" => {
            let items = tool_input["todos"]
                .as_array()?
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    let status = status_of(&t["status"]).flatten()?;
                    Some(TodoItem { id: (i + 1).to_string(), subject: subject(&t["content"])?, status })
                })
                .collect();
            Some(TodoChange::Replace { items })
        }
        _ => None,
    }
}

/// Claude Code の hook の入力 JSON から。関係の無いイベント、読めない形は None。
pub fn signal_from_hook(input: &Value) -> Option<HookSignal> {
    match input.get("hook_event_name")?.as_str()? {
        "UserPromptSubmit" => Some(HookSignal::Working),
        "Stop" | "PermissionRequest" => Some(HookSignal::Waiting),
        "Notification" => match input.get("notification_type").and_then(Value::as_str) {
            None => Some(HookSignal::Waiting),
            Some(t) if WAITING_NOTIFICATIONS.contains(&t) => Some(HookSignal::Waiting),
            Some(_) => None,
        },
        "SessionStart" => {
            let reset = matches!(input.get("source").and_then(Value::as_str), Some("startup" | "clear") | None);
            Some(HookSignal::SessionStart { reset })
        }
        "SessionEnd" => Some(HookSignal::SessionEnd),
        "PostToolUse" => todo_change(input).map(|change| HookSignal::Todo { change }),
        _ => None,
    }
}

/// "p12" → Some(12)。それ以外は None。
pub fn parse_terminal_id(id: &str) -> Option<u64> {
    let digits = id.strip_prefix('p')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AgentState {
    #[default]
    Idle,
    Working,
    Waiting,
}

#[derive(Clone, Debug, Default)]
pub struct PaneAgent {
    pub state: AgentState,
    pub todos: Vec<TodoItem>,
    /// タスクが最後に変わった順番（0 は変わっていない）。
    pub todo_seq: u64,
}

impl PaneAgent {
    fn change_todos(&mut self, change: TodoChange) {
        match change {
            TodoChange::Create { id, subject } => {
                self.todos.retain(|t| t.id != id);
                self.todos.push(TodoItem { id, subject, status: TodoStatus::Pending });
            }
            TodoChange::Update { id, status, subject } => {
                if let Some(t) = self.todos.iter_mut().find(|t| t.id == id) {
                    if let Some(s) = status {
                        t.status = s;
                    }
                    if let Some(s) = subject {
                        t.subject = s;
                    }
                }
            }
            TodoChange::Remove { id } => self.todos.retain(|t| t.id != id),
            TodoChange::Replace { items } => self.todos = items,
        }
    }
}

/// ペインの番号（PaneId::raw）ごとの状態。
#[derive(Debug, Default)]
pub struct Agents {
    panes: HashMap<u64, PaneAgent>,
    seq: u64,
}

impl Agents {
    /// 戻り値は、この信号で返事待ちになった（前は返事待ちでなかった）か。
    pub fn apply(&mut self, pane: u64, signal: HookSignal) -> bool {
        if signal == HookSignal::SessionEnd {
            self.panes.remove(&pane);
            return false;
        }
        let agent = self.panes.entry(pane).or_default();
        match signal {
            HookSignal::Working => agent.state = AgentState::Working,
            HookSignal::Waiting => {
                let was = agent.state;
                agent.state = AgentState::Waiting;
                return was != AgentState::Waiting;
            }
            HookSignal::SessionStart { reset } => {
                agent.state = AgentState::Idle;
                if reset && !agent.todos.is_empty() {
                    agent.todos.clear();
                    self.seq += 1;
                    agent.todo_seq = self.seq;
                }
            }
            HookSignal::Todo { change } => {
                agent.change_todos(change);
                self.seq += 1;
                agent.todo_seq = self.seq;
            }
            HookSignal::SessionEnd => unreachable!("先に扱った"),
        }
        false
    }

    /// 入力した。返事待ちなら Idle にし、true を返す。
    pub fn typed(&mut self, pane: u64) -> bool {
        match self.panes.get_mut(&pane) {
            Some(a) if a.state == AgentState::Waiting => {
                a.state = AgentState::Idle;
                true
            }
            _ => false,
        }
    }

    pub fn retain(&mut self, alive: impl Fn(u64) -> bool) {
        self.panes.retain(|p, _| alive(*p));
    }

    pub fn state(&self, pane: u64) -> AgentState {
        self.panes.get(&pane).map_or(AgentState::Idle, |a| a.state)
    }

    pub fn waiting_count(&self) -> usize {
        self.panes.values().filter(|a| a.state == AgentState::Waiting).count()
    }

    /// panes のうち、タスクが最後に変わったもののタスク（空なら None）。
    pub fn latest_todos(&self, panes: impl IntoIterator<Item = u64>) -> Option<&[TodoItem]> {
        panes
            .into_iter()
            .filter_map(|p| self.panes.get(&p))
            .filter(|a| !a.todos.is_empty())
            .max_by_key(|a| a.todo_seq)
            .map(|a| a.todos.as_slice())
    }
}

/// hook の登録に使うコマンド。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookCommand {
    /// 実行ファイルの絶対パス（args に "hook"）。
    Exec(String),
    /// シェルに渡す文字列（後ろに " hook" を付ける）。
    Shell(String),
}

/// settings.json に貼る `{"hooks": {...}}`。すべて async（Claude Code を待たせない）。
pub fn hook_config(cmd: &HookCommand) -> Value {
    let handler = match cmd {
        HookCommand::Exec(path) => json!({"type": "command", "command": path, "args": ["hook"], "async": true}),
        HookCommand::Shell(s) => json!({"type": "command", "command": format!("{s} hook"), "async": true}),
    };
    let plain = json!([{ "hooks": [handler.clone()] }]);
    json!({
        "hooks": {
            "UserPromptSubmit": plain,
            "Stop": plain,
            "Notification": plain,
            "PermissionRequest": plain,
            "SessionStart": plain,
            "SessionEnd": plain,
            "PostToolUse": [{ "matcher": "TaskCreate|TaskUpdate|TodoWrite", "hooks": [handler] }],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{AgentState, Agents, HookCommand, HookSignal, SUBJECT_MAX_CHARS, TodoChange, TodoItem, TodoStatus};
    use super::{hook_config, parse_terminal_id, signal_from_hook};
    use serde_json::json;

    #[test]
    fn prompt_submit_is_working() {
        let v = json!({"hook_event_name": "UserPromptSubmit", "prompt": "やって", "session_id": "s"});
        assert_eq!(signal_from_hook(&v), Some(HookSignal::Working));
    }

    #[test]
    fn stop_permission_and_notifications_wait() {
        for v in [
            json!({"hook_event_name": "Stop"}),
            json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": {}}),
            json!({"hook_event_name": "Notification", "notification_type": "permission_prompt", "message": "m"}),
            json!({"hook_event_name": "Notification", "notification_type": "idle_prompt"}),
            json!({"hook_event_name": "Notification", "message": "種類なし"}),
        ] {
            assert_eq!(signal_from_hook(&v), Some(HookSignal::Waiting), "{v}");
        }
        assert_eq!(signal_from_hook(&json!({"hook_event_name": "Notification", "notification_type": "auth_success"})), None);
        assert_eq!(signal_from_hook(&json!({"hook_event_name": "PreCompact"})), None);
        assert_eq!(signal_from_hook(&json!("文字列")), None);
    }

    #[test]
    fn session_events() {
        assert_eq!(
            signal_from_hook(&json!({"hook_event_name": "SessionStart", "source": "startup"})),
            Some(HookSignal::SessionStart { reset: true })
        );
        assert_eq!(
            signal_from_hook(&json!({"hook_event_name": "SessionStart", "source": "clear"})),
            Some(HookSignal::SessionStart { reset: true })
        );
        assert_eq!(
            signal_from_hook(&json!({"hook_event_name": "SessionStart", "source": "resume"})),
            Some(HookSignal::SessionStart { reset: false })
        );
        assert_eq!(signal_from_hook(&json!({"hook_event_name": "SessionEnd", "reason": "other"})), Some(HookSignal::SessionEnd));
    }

    #[test]
    fn task_tools_become_todo_changes() {
        let create = json!({"hook_event_name": "PostToolUse", "tool_name": "TaskCreate",
            "tool_input": {"subject": "設計", "description": "d", "activeForm": "設計中"},
            "tool_response": {"task": {"id": "3", "subject": "設計"}}});
        assert_eq!(
            signal_from_hook(&create),
            Some(HookSignal::Todo { change: TodoChange::Create { id: "3".into(), subject: "設計".into() } })
        );
        let update = json!({"hook_event_name": "PostToolUse", "tool_name": "TaskUpdate",
            "tool_input": {"taskId": "3", "status": "in_progress"}, "tool_response": {"success": true}});
        assert_eq!(
            signal_from_hook(&update),
            Some(HookSignal::Todo {
                change: TodoChange::Update { id: "3".into(), status: Some(TodoStatus::InProgress), subject: None }
            })
        );
        let delete = json!({"hook_event_name": "PostToolUse", "tool_name": "TaskUpdate", "tool_input": {"taskId": "3", "status": "deleted"}});
        assert_eq!(signal_from_hook(&delete), Some(HookSignal::Todo { change: TodoChange::Remove { id: "3".into() } }));
        let write = json!({"hook_event_name": "PostToolUse", "tool_name": "TodoWrite",
            "tool_input": {"todos": [{"content": "a", "status": "completed"}, {"content": "b", "status": "pending"}]}});
        assert_eq!(
            signal_from_hook(&write),
            Some(HookSignal::Todo {
                change: TodoChange::Replace {
                    items: vec![
                        TodoItem { id: "1".into(), subject: "a".into(), status: TodoStatus::Completed },
                        TodoItem { id: "2".into(), subject: "b".into(), status: TodoStatus::Pending },
                    ]
                }
            })
        );
        // 作った結果に id が無ければ読めない。
        assert_eq!(
            signal_from_hook(&json!({"hook_event_name": "PostToolUse", "tool_name": "TaskCreate", "tool_input": {"subject": "x"}})),
            None
        );
        assert_eq!(signal_from_hook(&json!({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_input": {}})), None);
    }

    #[test]
    fn numeric_task_id_becomes_string() {
        let create = json!({"hook_event_name": "PostToolUse", "tool_name": "TaskCreate",
            "tool_input": {"subject": "a"}, "tool_response": {"task": {"id": 7}}});
        assert_eq!(
            signal_from_hook(&create),
            Some(HookSignal::Todo { change: TodoChange::Create { id: "7".into(), subject: "a".into() } })
        );
    }

    #[test]
    fn big_prompt_signal_is_small() {
        let big = "あ".repeat(200_000);
        let v = json!({"hook_event_name": "UserPromptSubmit", "prompt": big});
        let s = serde_json::to_string(&signal_from_hook(&v).unwrap()).unwrap();
        assert!(s.len() < 100, "{}", s.len());
        let v = json!({"hook_event_name": "PostToolUse", "tool_name": "TaskCreate",
            "tool_input": {"subject": "長".repeat(5000)}, "tool_response": {"task": {"id": "1"}}});
        let Some(HookSignal::Todo { change: TodoChange::Create { subject, .. } }) = signal_from_hook(&v) else { panic!() };
        assert_eq!(subject.chars().count(), SUBJECT_MAX_CHARS);
    }

    #[test]
    fn terminal_id_parses() {
        assert_eq!(parse_terminal_id("p12"), Some(12));
        assert_eq!(parse_terminal_id("p"), None);
        assert_eq!(parse_terminal_id("12"), None);
        assert_eq!(parse_terminal_id("p1x"), None);
        assert_eq!(parse_terminal_id("p+1"), None);
    }

    #[test]
    fn agents_wait_and_typing_clears() {
        let mut a = Agents::default();
        assert!(!a.apply(1, HookSignal::Working));
        assert!(a.apply(1, HookSignal::Waiting));
        assert!(!a.apply(1, HookSignal::Waiting), "続けて来ても、新しく待ちになったとは数えない");
        assert_eq!(a.waiting_count(), 1);
        assert!(a.typed(1));
        assert_eq!(a.state(1), AgentState::Idle);
        assert!(!a.typed(1));
        a.apply(1, HookSignal::Working);
        assert!(!a.typed(1), "作業中は入力で変えない");
        assert_eq!(a.state(1), AgentState::Working);
    }

    #[test]
    fn agents_track_todos_and_latest() {
        let mut a = Agents::default();
        a.apply(1, HookSignal::Todo { change: TodoChange::Create { id: "1".into(), subject: "a".into() } });
        a.apply(2, HookSignal::Todo { change: TodoChange::Create { id: "1".into(), subject: "b".into() } });
        a.apply(
            1,
            HookSignal::Todo { change: TodoChange::Update { id: "1".into(), status: Some(TodoStatus::Completed), subject: None } },
        );
        let latest = a.latest_todos([1, 2]).unwrap();
        assert_eq!(latest[0].subject, "a");
        assert_eq!(latest[0].status, TodoStatus::Completed);
        assert_eq!(a.latest_todos([2]).unwrap()[0].subject, "b");
        a.apply(1, HookSignal::Todo { change: TodoChange::Remove { id: "1".into() } });
        assert_eq!(a.latest_todos([1]), None);
        a.apply(2, HookSignal::SessionStart { reset: true });
        assert_eq!(a.latest_todos([2]), None);
        a.apply(2, HookSignal::Waiting);
        a.retain(|p| p != 2);
        assert_eq!(a.waiting_count(), 0);
        assert_eq!(a.latest_todos([3]), None);
    }

    #[test]
    fn session_end_forgets_pane() {
        let mut a = Agents::default();
        a.apply(1, HookSignal::Waiting);
        a.apply(1, HookSignal::SessionEnd);
        assert_eq!(a.state(1), AgentState::Idle);
        assert_eq!(a.waiting_count(), 0);
    }

    #[test]
    fn unknown_pane_is_ignored() {
        let mut a = Agents::default();
        assert!(!a.typed(99));
        assert_eq!(a.state(99), AgentState::Idle);
    }

    #[test]
    fn closed_pane_forgets_state() {
        let mut a = Agents::default();
        a.apply(5, HookSignal::Waiting);
        a.retain(|_| false);
        assert_eq!(a.state(5), AgentState::Idle);
        assert_eq!(a.waiting_count(), 0);
    }

    #[test]
    fn hook_config_shapes() {
        let c = hook_config(&HookCommand::Exec("C:/fude/fude.exe".into()));
        let h = &c["hooks"];
        for ev in ["UserPromptSubmit", "Stop", "Notification", "PermissionRequest", "SessionStart", "SessionEnd", "PostToolUse"] {
            let entry = &h[ev][0]["hooks"][0];
            assert_eq!(entry["type"], "command", "{ev}");
            assert_eq!(entry["command"], "C:/fude/fude.exe");
            assert_eq!(entry["args"], json!(["hook"]));
            assert_eq!(entry["async"], true);
        }
        assert_eq!(h["PostToolUse"][0]["matcher"], "TaskCreate|TaskUpdate|TodoWrite");
        assert!(h["Stop"][0].get("matcher").is_none());
        let s = hook_config(&HookCommand::Shell("~/.local/bin/fude".into()));
        assert_eq!(s["hooks"]["Stop"][0]["hooks"][0]["command"], "~/.local/bin/fude hook");
        assert!(s["hooks"]["Stop"][0]["hooks"][0].get("args").is_none());
    }
}
