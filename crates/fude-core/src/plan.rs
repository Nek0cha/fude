//! Superpowers のプランと台帳を読み、路線図（設計書 §4.1、§5.8）の形にする。
//!
//! Superpowers 7 はプランのチェックボックスに印を付けず、進み具合を台帳
//! （`.superpowers/sdd/<名前>/progress.md` の `Task <N>: complete`）に書く。そのため完了は台帳とチェックボックスの両方から求める。

use crate::claude::{TodoItem, TodoStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanTask {
    pub number: Option<u32>,
    pub title: String,
    /// 見出しの行（1 始まり）。
    pub line: u32,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub title: String,
    pub tasks: Vec<PlanTask>,
    /// チェックボックスに 1 つでも [x] がある。
    pub ticked: bool,
}

/// 見出し（1 始まりの行、`#` の数、文字）。
struct Heading {
    line: u32,
    level: usize,
    text: String,
}

/// `#` が 1〜4 個と空白で始まる行を見出しとして読む。
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=4).contains(&level) {
        return None;
    }
    let rest = &line[level..];
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((level, rest.trim()))
}

/// チェックボックスの行なら Some(印があるか)。
fn checkbox(line: &str) -> Option<bool> {
    let t = line.trim_start();
    let rest = t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))?;
    match rest.get(..3)? {
        "[ ]" => Some(false),
        "[x]" | "[X]" => Some(true),
        _ => None,
    }
}

/// `Task 3: 題名`、`タスク 1：題名`、`Task 2. 題名` を（番号, 題名）にする。
fn task_heading(text: &str) -> Option<(u32, String)> {
    let rest = text.strip_prefix("Task").or_else(|| text.strip_prefix("タスク"))?.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let after = &rest[digits.len()..];
    let mut chars = after.chars();
    let title = match chars.next() {
        None => "",
        Some(':' | '：' | '.' | ' ' | '\t') => chars.as_str(),
        Some(_) => return None,
    };
    Some((digits.parse().ok()?, title.trim().to_string()))
}

/// name は題名が無いときに使うファイル名（拡張子なし）。
pub fn parse_plan(text: &str, name: &str) -> Plan {
    let mut headings = Vec::new();
    let mut boxes: Vec<(u32, bool)> = Vec::new();
    // コードブロックの印（``` か ~~~）。中にいる間は Some。
    let mut fence: Option<&str> = None;
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let n = i as u32 + 1;
        let trimmed = line.trim_start();
        let marker = ["```", "~~~"].into_iter().find(|m| trimmed.starts_with(m));
        match (fence, marker) {
            (Some(open), Some(m)) if open == m => {
                fence = None;
                continue;
            }
            (Some(_), _) => continue,
            (None, Some(m)) => {
                fence = Some(m);
                continue;
            }
            (None, None) => {}
        }
        if let Some((level, text)) = heading(line) {
            headings.push(Heading { line: n, level, text: text.to_string() });
        } else if let Some(checked) = checkbox(line) {
            boxes.push((n, checked));
        }
    }
    let title = headings
        .iter()
        .find(|h| h.level == 1)
        .map(|h| {
            let t = h.text.trim();
            let t = t.strip_suffix("Implementation Plan").or_else(|| t.strip_suffix("実装計画")).unwrap_or(t);
            t.trim().to_string()
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| name.to_string());
    let ticked = boxes.iter().any(|&(_, c)| c);

    // 範囲の中のチェックボックスから、完了か（1 つ以上あり、全部に印）を求める。
    let done_in = |from: u32, to: u32| {
        let inside: Vec<bool> = boxes.iter().filter(|&&(l, _)| l > from && l < to).map(|&(_, c)| c).collect();
        !inside.is_empty() && inside.iter().all(|&c| c)
    };
    let task_like: Vec<(usize, u32, String)> = headings
        .iter()
        .enumerate()
        .filter(|(_, h)| (2..=4).contains(&h.level))
        .filter_map(|(i, h)| task_heading(&h.text).map(|(num, title)| (i, num, title)))
        .collect();
    let tasks = if !task_like.is_empty() {
        task_like
            .into_iter()
            .map(|(i, number, title)| {
                let h = &headings[i];
                // 同じか浅い見出しまでを、そのタスクの範囲にする。
                let end = headings[i + 1..].iter().find(|n| n.level <= h.level).map_or(u32::MAX, |n| n.line);
                PlanTask { number: Some(number), title, line: h.line, done: done_in(h.line, end) }
            })
            .collect()
    } else {
        // Task の見出しが無ければ、チェックボックスを含む ## / ### の見出し（次の見出しまで）をタスクにする。
        headings
            .iter()
            .enumerate()
            .filter(|(_, h)| (2..=3).contains(&h.level))
            .filter_map(|(i, h)| {
                let end = headings.get(i + 1).map_or(u32::MAX, |n| n.line);
                let has_box = boxes.iter().any(|&(l, _)| l > h.line && l < end);
                has_box.then(|| PlanTask { number: None, title: h.text.clone(), line: h.line, done: done_in(h.line, end) })
            })
            .collect()
    };
    Plan { title, tasks, ticked }
}

/// 台帳の `Task <N>: complete` の N。
pub fn ledger_done(ledger: &str) -> Vec<u32> {
    ledger
        .lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Task ")?;
            let (num, after) = rest.split_once(':')?;
            after.trim_start().starts_with("complete").then(|| num.trim().parse().ok()).flatten()
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StationState {
    Done,
    Current,
    Todo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Station {
    pub label: String,
    pub state: StationState,
    /// プランのとき、開く行。
    pub line: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub title: String,
    pub stations: Vec<Station>,
    /// プランのとき、そのファイルの相対パス。
    pub plan_rel: Option<String>,
}

impl Route {
    /// （完了の数, 全部の数）。
    pub fn progress(&self) -> (usize, usize) {
        (self.stations.iter().filter(|s| s.state == StationState::Done).count(), self.stations.len())
    }

    pub fn current(&self) -> Option<usize> {
        self.stations.iter().position(|s| s.state == StationState::Current)
    }
}

/// 台帳も印も無いプランを、書いた直後とみなして出す期間（更新からの時間）。
/// Superpowers は終わると台帳を消すため、終わったプランと書いただけのプランは見分けられない。期間で区切る。
pub const RECENT_PLAN_MS: u64 = 24 * 60 * 60 * 1000;

/// プランの更新が RECENT_PLAN_MS 以内か。更新時刻が未来（時計のずれ）なら最近とみなす。
pub fn plan_is_recent(plan_mtime_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(plan_mtime_ms) < RECENT_PLAN_MS
}

/// 台帳が無く、印も無いプランは、最近（recent）でなければ None。タスクが 0 個も None。実行中は最初の未完了。
pub fn plan_route(rel: &str, text: &str, ledger: Option<&str>, recent: bool) -> Option<Route> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let name = name.strip_suffix(".md").unwrap_or(name);
    let plan = parse_plan(text, name);
    if (ledger.is_none() && !plan.ticked && !recent) || plan.tasks.is_empty() {
        return None;
    }
    let done = ledger.map(ledger_done).unwrap_or_default();
    let mut current_set = false;
    let stations = plan
        .tasks
        .into_iter()
        .map(|t| {
            let is_done = t.done || t.number.is_some_and(|n| done.contains(&n));
            let state = if is_done {
                StationState::Done
            } else if !current_set {
                current_set = true;
                StationState::Current
            } else {
                StationState::Todo
            };
            Station { label: t.title, state, line: Some(t.line) }
        })
        .collect();
    Some(Route { title: plan.title, stations, plan_rel: Some(rel.to_string()) })
}

/// 題名は "Todo"。実行中は最初の in_progress、無ければ最初の pending。空なら None。
pub fn todo_route(todos: &[TodoItem]) -> Option<Route> {
    if todos.is_empty() {
        return None;
    }
    let current = todos
        .iter()
        .position(|t| t.status == TodoStatus::InProgress)
        .or_else(|| todos.iter().position(|t| t.status == TodoStatus::Pending));
    let stations = todos
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let state = match t.status {
                TodoStatus::Completed => StationState::Done,
                _ if Some(i) == current => StationState::Current,
                _ => StationState::Todo,
            };
            Station { label: t.subject.clone(), state, line: None }
        })
        .collect();
    Some(Route { title: "Todo".to_string(), stations, plan_rel: None })
}

#[cfg(test)]
mod tests {
    use super::{RECENT_PLAN_MS, StationState, ledger_done, parse_plan, plan_is_recent, plan_route, todo_route};

    const PLAN: &str = "# fude 計画9: 仕上げ Implementation Plan\n\n## Global Constraints\n\n- [ ] 例ではない\n\n### Task 1: 一つ目\n\n- [x] **Step 1: a**\n- [x] **Step 2: b**\n\n### Task 2: 二つ目\n\n- [x] **Step 1: a**\n- [ ] **Step 2: b**\n\n### Task 3: 三つ目\n\n- [ ] **Step 1: a**\n";

    #[test]
    fn parses_tasks_titles_and_lines() {
        let p = parse_plan(PLAN, "x");
        assert_eq!(p.title, "fude 計画9: 仕上げ");
        assert_eq!(
            p.tasks.iter().map(|t| (t.number, t.title.as_str(), t.done)).collect::<Vec<_>>(),
            vec![(Some(1), "一つ目", true), (Some(2), "二つ目", false), (Some(3), "三つ目", false)]
        );
        assert_eq!(p.tasks[0].line, 7);
        assert!(p.ticked);
    }

    #[test]
    fn fenced_code_is_ignored() {
        let text = "# P\n\n### Task 1: 本物\n\n```bash\n# コメント\n### Task 9: 偽物\n- [x] 偽の印\n```\n\n~~~\n- [x] これも偽\n~~~\n- [ ] **Step 1**\n";
        let p = parse_plan(text, "x");
        assert_eq!(p.tasks.len(), 1);
        assert!(!p.tasks[0].done);
        assert!(!p.ticked);
    }

    #[test]
    fn heading_variants() {
        let text = "# 計画\n## タスク 1：準備\n- [ ] a\n#### Task 2. 本体\n- [ ] b\n";
        let p = parse_plan(text, "x");
        assert_eq!(
            p.tasks.iter().map(|t| (t.number, t.title.as_str())).collect::<Vec<_>>(),
            vec![(Some(1), "準備"), (Some(2), "本体")]
        );
    }

    #[test]
    fn falls_back_to_headings_with_checkboxes() {
        let text = "## 準備\n- [x] a\n## メモ\n本文\n### 本体\n- [ ] b\n";
        let p = parse_plan(text, "my-plan");
        assert_eq!(p.title, "my-plan");
        assert_eq!(
            p.tasks.iter().map(|t| (t.title.as_str(), t.done)).collect::<Vec<_>>(),
            vec![("準備", true), ("本体", false)]
        );
    }

    #[test]
    fn crlf_plan_and_ledger() {
        let text = PLAN.replace('\n', "\r\n");
        let p = parse_plan(&text, "x");
        assert_eq!(p.title, "fude 計画9: 仕上げ");
        assert_eq!(p.tasks[1].title, "二つ目");
        let ledger = "# SDD ledger — plan: docs/x.md\r\nPre-flight: no shared interfaces\r\nTask 1: complete (commits a..b, tests: cargo test → ok)\r\nTask 2: Ruling: x\r\nTask 3: complete (commits c..d)\r\n";
        assert_eq!(ledger_done(ledger), vec![1, 3]);
    }

    #[test]
    fn plan_route_uses_ledger() {
        let text = "# P\n### Task 1: a\n- [ ] s\n### Task 2: b\n- [ ] s\n### Task 3: c\n- [ ] s\n";
        let r = plan_route("docs/superpowers/plans/p.md", text, Some("Task 1: complete (x)\n"), false).unwrap();
        assert_eq!(r.progress(), (1, 3));
        assert_eq!(r.current(), Some(1));
        assert_eq!(
            r.stations.iter().map(|s| s.state).collect::<Vec<_>>(),
            vec![StationState::Done, StationState::Current, StationState::Todo]
        );
        assert_eq!(r.stations[1].line, Some(4));
        assert_eq!(r.plan_rel.as_deref(), Some("docs/superpowers/plans/p.md"));
        // 台帳が空でも、台帳があれば出す。
        assert_eq!(plan_route("p.md", text, Some("# SDD ledger — plan: p.md\n"), false).unwrap().progress(), (0, 3));
    }

    #[test]
    fn finished_plan_without_ledger_is_hidden() {
        let text = "# P\n### Task 1: a\n- [ ] s\n";
        assert_eq!(plan_route("p.md", text, None, false), None);
        assert_eq!(plan_route("p.md", "# P\n本文だけ\n", Some(""), false), None);
        let all = plan_route("p.md", "# P\n### Task 1: a\n- [x] s\n", None, false).unwrap();
        assert_eq!(all.progress(), (1, 1));
        assert_eq!(all.current(), None);
    }

    /// 書いた直後のプランや、git の外（Superpowers の台帳ができない）のプランも、最近のものなら 0/N で出す。
    #[test]
    fn recent_plan_without_ledger_is_shown() {
        let text = "# P\n### Task 1: a\n- [ ] s\n### Task 2: b\n- [ ] s\n";
        let r = plan_route("p.md", text, None, true).unwrap();
        assert_eq!(r.progress(), (0, 2));
        assert_eq!(r.current(), Some(0));
        // タスクが無いプランは、最近でも出さない。
        assert_eq!(plan_route("p.md", "# P\n本文だけ\n", None, true), None);
    }

    #[test]
    fn plan_recency_window() {
        let day = RECENT_PLAN_MS;
        assert!(plan_is_recent(1_000, 1_000));
        assert!(plan_is_recent(1_000, 1_000 + day - 1));
        assert!(!plan_is_recent(1_000, 1_000 + day));
        // 時計のずれで更新時刻が未来でも、最近として扱う。
        assert!(plan_is_recent(5_000, 1_000));
    }

    #[test]
    fn todo_route_picks_current() {
        use crate::claude::{TodoItem, TodoStatus};
        let item = |id: &str, s| TodoItem { id: id.into(), subject: format!("t{id}"), status: s };
        let r = todo_route(&[item("1", TodoStatus::Completed), item("2", TodoStatus::Pending), item("3", TodoStatus::InProgress)])
            .unwrap();
        assert_eq!(r.title, "Todo");
        assert_eq!(r.current(), Some(2));
        assert_eq!(r.progress(), (1, 3));
        assert_eq!(r.stations[1].state, StationState::Todo);
        assert_eq!(todo_route(&[item("1", TodoStatus::Completed), item("2", TodoStatus::Pending)]).unwrap().current(), Some(1));
        assert_eq!(todo_route(&[]), None);
    }
}
