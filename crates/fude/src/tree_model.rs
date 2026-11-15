//! ファイルツリーの中身（設計書 §5.5）。どのフォルダを読んだか、開いているか、絞り込みと「変更だけ」から、
//! 表示する行を作る。GUI には依らない。

use std::collections::{BTreeMap, HashMap, HashSet};

use fude_fs::git::{GitMark, GitStatus};
use fude_fs::{DirEntry, RelPath};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TreeRow {
    pub rel: RelPath,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub mark: Option<GitMark>,
    /// gitignore などで外されている（計画8d）。薄く描く。
    pub ignored: bool,
}

/// ツリーに描く 1 行。名前の入力欄（計画8b）も行として並べる。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RowSlot {
    Entry(TreeRow),
    Edit { depth: usize, kind: crate::tree_menu::EditKind },
}

/// rows に名前の入力欄を差し込む。名前の変更は対象の行を置き換え、新規は作り先のフォルダの行のすぐ下
/// （ルートならいちばん上）に 1 段深く差し込む。対象の行が見えていなければ差し込まない。
pub(crate) fn rows_with_edit(rows: Vec<TreeRow>, edit: Option<(&crate::tree_menu::EditKind, &RelPath)>) -> Vec<RowSlot> {
    use crate::tree_menu::EditKind;
    let Some((&kind, target)) = edit else { return rows.into_iter().map(RowSlot::Entry).collect() };
    let mut out = Vec::with_capacity(rows.len() + 1);
    if kind != EditKind::Rename && target.is_root() {
        out.push(RowSlot::Edit { depth: 0, kind });
    }
    for row in rows {
        if row.rel != *target {
            out.push(RowSlot::Entry(row));
        } else if kind == EditKind::Rename {
            out.push(RowSlot::Edit { depth: row.depth, kind });
        } else {
            let depth = row.depth + 1;
            out.push(RowSlot::Entry(row));
            out.push(RowSlot::Edit { depth, kind });
        }
    }
    out
}

#[derive(Default)]
pub(crate) struct TreeModel {
    /// 読んだフォルダ（相対パス）の子。
    children: HashMap<String, Vec<DirEntry>>,
    /// 監視で変わったと知らされ、読み直しを待つフォルダ。
    stale: HashSet<String>,
    expanded: HashSet<String>,
    filter: String,
    all_files: Option<fude_fs::WalkedFiles>,
    changed_only: bool,
}

/// パスの集まりから作る木（絞り込みと「変更だけ」で使う）。
#[derive(Default)]
struct PathTree {
    dirs: BTreeMap<String, PathTree>,
    /// ファイルの名前と、外したものか。
    files: Vec<(String, bool)>,
}

impl PathTree {
    fn insert(&mut self, parts: &[&str], ignored: bool) {
        match parts {
            [] => {}
            [file] => self.files.push((file.to_string(), ignored)),
            [dir, rest @ ..] => self.dirs.entry(dir.to_string()).or_default().insert(rest, ignored),
        }
    }

    /// 中のファイルがすべて外したものか（フォルダの行を薄くするか）。
    fn all_ignored(&self) -> bool {
        self.files.iter().all(|(_, ignored)| *ignored) && self.dirs.values().all(PathTree::all_ignored)
    }

    fn rows(&self, prefix: &str, depth: usize, status: Option<&GitStatus>, out: &mut Vec<TreeRow>) {
        for (name, sub) in &self.dirs {
            let rel = join(prefix, name);
            out.push(row(&rel, name, depth, true, true, status, sub.all_ignored()));
            sub.rows(&rel, depth + 1, status, out);
        }
        let mut files = self.files.clone();
        files.sort();
        files.dedup_by(|a, b| a.0 == b.0);
        for (name, ignored) in files {
            let rel = join(prefix, &name);
            out.push(row(&rel, &name, depth, false, false, status, ignored));
        }
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") }
}

fn row(rel: &str, name: &str, depth: usize, is_dir: bool, expanded: bool, status: Option<&GitStatus>, ignored: bool) -> TreeRow {
    let rel = RelPath::new(rel).unwrap_or_else(|_| RelPath::root());
    let mark = if is_dir { None } else { status.and_then(|s| s.mark(&rel)) };
    TreeRow { rel, name: name.to_string(), depth, is_dir, expanded, mark, ignored }
}

fn parent_of(rel: &RelPath) -> String {
    rel.as_str().rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default()
}

impl TreeModel {
    /// dir の子を読む必要があるか（まだ読んでいない、または読み直しを待っている）。
    pub fn needs_load(&self, dir: &RelPath) -> bool {
        !self.children.contains_key(dir.as_str()) || self.stale.contains(dir.as_str())
    }

    pub fn set_children(&mut self, dir: &RelPath, entries: Vec<DirEntry>) {
        self.stale.remove(dir.as_str());
        self.children.insert(dir.as_str().to_string(), entries);
    }

    /// 開閉を切り替える。開いたら true。
    pub fn toggle(&mut self, dir: &RelPath) -> bool {
        let key = dir.as_str().to_string();
        if self.expanded.remove(&key) {
            false
        } else {
            self.expanded.insert(key);
            true
        }
    }

    /// dir を開いた状態にする（ルートは常に開いている）。
    pub fn expand(&mut self, dir: &RelPath) {
        if !dir.is_root() {
            self.expanded.insert(dir.as_str().to_string());
        }
    }

    /// dir と、その祖先をすべて開く。読む必要のあるフォルダ（浅い順）を返す。
    pub fn reveal(&mut self, dir: &RelPath) -> Vec<RelPath> {
        let mut need = Vec::new();
        let mut path = String::new();
        for part in dir.as_str().split('/').filter(|p| !p.is_empty()) {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            let Ok(rel) = RelPath::new(&path) else { break };
            self.expand(&rel);
            if self.needs_load(&rel) {
                need.push(rel);
            }
        }
        need
    }

    /// ファイル名で絞り込む（大文字小文字を区別しない部分一致）。all_files は全ファイルの一覧（無ければ、届くまで何も出さない）。
    pub fn set_filter(&mut self, query: &str, all_files: Option<&fude_fs::WalkedFiles>) {
        self.filter = query.trim().to_lowercase();
        if let Some(all) = all_files {
            self.all_files = Some(all.clone());
        }
    }

    pub fn filtering(&self) -> bool {
        !self.filter.is_empty()
    }

    pub fn set_changed_only(&mut self, on: bool) {
        self.changed_only = on;
    }

    pub fn changed_only(&self) -> bool {
        self.changed_only
    }

    /// 変わったパスから、読み直しが要るフォルダ（読んだことのある親）を返し、読み直し待ちにする。全ファイルの一覧も捨てる。
    pub fn invalidate(&mut self, changed: &[RelPath]) -> Vec<RelPath> {
        self.all_files = None;
        let mut dirs: Vec<String> = changed.iter().map(parent_of).filter(|d| self.children.contains_key(d)).collect();
        dirs.sort();
        dirs.dedup();
        for d in &dirs {
            self.stale.insert(d.clone());
        }
        dirs.iter().filter_map(|d| RelPath::new(d).ok()).collect()
    }

    /// 開いているのに、まだ読んでいない（または読み直し待ちの）フォルダ。浅い順。ProjectFs が後から
    /// 用意できたときに、開いたまま空のフォルダを残さないために使う。
    pub fn unloaded_expanded(&self) -> Vec<RelPath> {
        let mut dirs: Vec<&String> = self.expanded.iter().filter(|d| !self.children.contains_key(*d) || self.stale.contains(*d)).collect();
        dirs.sort_by_key(|d| (d.matches('/').count(), d.as_str()));
        dirs.into_iter().filter_map(|d| RelPath::new(d).ok()).collect()
    }

    pub fn has_file_list(&self) -> bool {
        self.all_files.is_some()
    }

    pub fn rows(&self, status: Option<&GitStatus>) -> Vec<TreeRow> {
        let mut out = Vec::new();
        if self.filtering() {
            let Some(all) = &self.all_files else { return out };
            let mut tree = PathTree::default();
            let files = all.tracked.iter().map(|f| (f, false)).chain(all.ignored.iter().map(|f| (f, true)));
            for (f, ignored) in files {
                let name = f.as_str().rsplit('/').next().unwrap_or_default();
                if name.to_lowercase().contains(&self.filter) {
                    tree.insert(&f.as_str().split('/').collect::<Vec<_>>(), ignored);
                }
            }
            tree.rows("", 0, status, &mut out);
            return out;
        }
        if self.changed_only {
            let mut tree = PathTree::default();
            for (p, _) in status.map(|s| s.entries.as_slice()).unwrap_or_default() {
                tree.insert(&p.as_str().split('/').collect::<Vec<_>>(), false);
            }
            tree.rows("", 0, status, &mut out);
            return out;
        }
        self.dir_rows("", 0, status, false, &mut out);
        out
    }

    /// parent_ignored は dir が外されているか。外したフォルダの子は、すべて外したものとして描く。
    fn dir_rows(&self, dir: &str, depth: usize, status: Option<&GitStatus>, parent_ignored: bool, out: &mut Vec<TreeRow>) {
        let Some(entries) = self.children.get(dir) else { return };
        for e in entries {
            let rel = join(dir, &e.name);
            let expanded = e.is_dir && self.expanded.contains(&rel);
            let ignored = parent_ignored || e.ignored;
            out.push(row(&rel, &e.name, depth, e.is_dir, expanded, status, ignored));
            if expanded {
                self.dir_rows(&rel, depth + 1, status, ignored, out);
            }
        }
    }

    /// 外す規則が変わったとき。読んだフォルダをすべて読み直し待ちにして返し（浅い順）、全ファイルの一覧も捨てる。
    pub fn invalidate_all(&mut self) -> Vec<RelPath> {
        self.all_files = None;
        let mut dirs: Vec<&String> = self.children.keys().collect();
        dirs.sort_by_key(|d| (if d.is_empty() { 0 } else { d.matches('/').count() + 1 }, d.as_str()));
        let dirs: Vec<RelPath> = dirs.into_iter().filter_map(|d| RelPath::new(d).ok()).collect();
        for d in &dirs {
            self.stale.insert(d.as_str().to_string());
        }
        dirs
    }
}

#[cfg(test)]
mod tests {
    use super::{RowSlot, TreeModel, TreeRow, rows_with_edit};
    use crate::tree_menu::EditKind;
    use fude_fs::git::{GitMark, GitStatus};
    use fude_fs::{DirEntry, RelPath};

    fn rel(p: &str) -> RelPath {
        RelPath::new(p).unwrap()
    }

    fn entry(name: &str, is_dir: bool) -> DirEntry {
        DirEntry { name: name.into(), is_dir, ignored: false }
    }

    fn shape(rows: &[TreeRow]) -> Vec<(String, usize, bool)> {
        rows.iter().map(|r| (r.rel.as_str().to_string(), r.depth, r.is_dir)).collect()
    }

    #[test]
    fn rows_expand_lazily() {
        let mut m = TreeModel::default();
        assert!(m.needs_load(&RelPath::root()));
        m.set_children(&RelPath::root(), vec![entry("src", true), entry("a.md", false)]);
        assert_eq!(shape(&m.rows(None)), vec![("src".into(), 0, true), ("a.md".into(), 0, false)]);
        assert!(m.toggle(&rel("src")));
        assert!(m.needs_load(&rel("src")));
        assert_eq!(m.rows(None).len(), 2);
        m.set_children(&rel("src"), vec![entry("ui", true), entry("main.rs", false)]);
        assert_eq!(
            shape(&m.rows(None)),
            vec![("src".into(), 0, true), ("src/ui".into(), 1, true), ("src/main.rs".into(), 1, false), ("a.md".into(), 0, false)]
        );
        assert!(m.rows(None)[0].expanded);
        assert!(!m.toggle(&rel("src")));
        assert_eq!(m.rows(None).len(), 2);
        m.expand(&rel("src"));
        m.expand(&rel("src"));
        assert_eq!(m.rows(None).len(), 4);
    }

    #[test]
    fn filter_shows_matches_with_ancestors() {
        let mut m = TreeModel::default();
        m.set_children(&RelPath::root(), vec![entry("src", true), entry("README.md", false)]);
        let all = fude_fs::WalkedFiles { tracked: vec![rel("README.md"), rel("src/main.rs"), rel("src/ui/Button.tsx")], ignored: vec![] };
        m.set_filter("button", Some(&all));
        let rows = m.rows(None);
        assert_eq!(shape(&rows), vec![("src".into(), 0, true), ("src/ui".into(), 1, true), ("src/ui/Button.tsx".into(), 2, false)]);
        assert!(rows[0].expanded && rows[1].expanded);
        m.set_filter("", None);
        assert_eq!(m.rows(None).len(), 2);
    }

    #[test]
    fn filter_without_file_list_shows_nothing_yet() {
        let mut m = TreeModel::default();
        m.set_children(&RelPath::root(), vec![entry("a.rs", false)]);
        m.set_filter("a", None);
        assert!(m.rows(None).is_empty());
        assert!(m.filtering());
    }

    #[test]
    fn changed_only_includes_deleted() {
        let mut m = TreeModel::default();
        m.set_children(&RelPath::root(), vec![entry("src", true), entry("x.rs", false)]);
        let status = GitStatus { entries: vec![(rel("old.rs"), GitMark::Deleted), (rel("src/a.rs"), GitMark::Modified)] };
        m.set_changed_only(true);
        let rows = m.rows(Some(&status));
        assert_eq!(shape(&rows), vec![("src".into(), 0, true), ("src/a.rs".into(), 1, false), ("old.rs".into(), 0, false)]);
        assert_eq!(rows[2].mark, Some(GitMark::Deleted));
    }

    #[test]
    fn marks_come_from_status() {
        let mut m = TreeModel::default();
        m.set_children(&RelPath::root(), vec![entry("a.rs", false), entry("b.rs", false)]);
        let status = GitStatus { entries: vec![(rel("a.rs"), GitMark::Modified)] };
        let rows = m.rows(Some(&status));
        assert_eq!(rows[0].mark, Some(GitMark::Modified));
        assert_eq!(rows[1].mark, None);
    }

    #[test]
    fn invalidate_marks_dirs_for_reload() {
        let mut m = TreeModel::default();
        m.set_children(&RelPath::root(), vec![entry("src", true)]);
        m.toggle(&rel("src"));
        m.set_children(&rel("src"), vec![entry("a.rs", false)]);
        assert!(!m.needs_load(&rel("src")));
        let dirs = m.invalidate(&[rel("src/b.rs"), rel("other/x.rs")]);
        assert_eq!(dirs, vec![rel("src")]);
        assert!(m.needs_load(&rel("src")));
        // 読み直すまでは古い子を出し続ける（ちらつかないように）。
        assert_eq!(m.rows(None).len(), 2);
    }

    fn slot_row(p: &str, depth: usize, is_dir: bool) -> TreeRow {
        let rel = rel(p);
        TreeRow { name: p.rsplit('/').next().unwrap().into(), rel, depth, is_dir, expanded: is_dir, mark: None, ignored: false }
    }

    /// 作り先のフォルダと祖先をすべて開き、読む必要のあるものを返す（絞り込みからの新規、計画8b）。
    #[test]
    fn reveal_opens_ancestors() {
        let mut m = TreeModel::default();
        m.set_children(&RelPath::root(), vec![entry("src", true)]);
        let need = m.reveal(&rel("src/deep"));
        assert_eq!(need, vec![rel("src"), rel("src/deep")]);
        m.set_children(&rel("src"), vec![entry("deep", true)]);
        m.set_children(&rel("src/deep"), vec![]);
        let rows = m.rows(None);
        assert!(rows.iter().any(|r| r.rel == rel("src/deep") && r.expanded));
        assert!(m.reveal(&rel("src/deep")).is_empty());
        assert!(m.reveal(&RelPath::root()).is_empty());
    }

    #[test]
    fn edit_slot_replaces_renamed_row() {
        let rows = vec![slot_row("docs", 0, true), slot_row("docs/a.md", 1, false), slot_row("docs/b.md", 1, false)];
        let slots = rows_with_edit(rows, Some((&EditKind::Rename, &rel("docs/a.md"))));
        assert_eq!(slots.len(), 3);
        assert!(matches!(slots[1], RowSlot::Edit { depth: 1, kind: EditKind::Rename }));
        assert!(matches!(&slots[2], RowSlot::Entry(r) if r.rel == rel("docs/b.md")));
    }

    #[test]
    fn new_item_slot_goes_under_folder_or_top() {
        let rows = vec![slot_row("docs", 0, true), slot_row("docs/a.md", 1, false), slot_row("z.md", 0, false)];
        let slots = rows_with_edit(rows.clone(), Some((&EditKind::NewFile, &rel("docs"))));
        assert_eq!(slots.len(), 4);
        assert!(matches!(slots[1], RowSlot::Edit { depth: 1, kind: EditKind::NewFile }));
        let slots = rows_with_edit(rows.clone(), Some((&EditKind::NewFolder, &RelPath::root())));
        assert!(matches!(slots[0], RowSlot::Edit { depth: 0, kind: EditKind::NewFolder }));
        let slots = rows_with_edit(rows, None);
        assert_eq!(slots.len(), 3);
        assert!(slots.iter().all(|s| matches!(s, RowSlot::Entry(_))));
    }

    fn ignored_entry(name: &str, is_dir: bool) -> DirEntry {
        DirEntry { name: name.into(), is_dir, ignored: true }
    }

    #[test]
    fn rows_carry_ignored_mark() {
        let mut m = TreeModel::default();
        m.set_children(&RelPath::root(), vec![ignored_entry("docs", true), entry("src", true)]);
        let rows = m.rows(None);
        assert!(rows[0].ignored);
        assert!(!rows[1].ignored);
    }

    #[test]
    fn children_of_ignored_dir_are_ignored() {
        let mut m = TreeModel::default();
        m.set_children(&RelPath::root(), vec![ignored_entry("docs", true)]);
        m.toggle(&rel("docs"));
        m.set_children(&rel("docs"), vec![entry("a.md", false)]);
        let rows = m.rows(None);
        assert_eq!(rows.len(), 2);
        assert!(rows[1].ignored);
    }

    #[test]
    fn filter_rows_mark_ignored_files_and_dirs() {
        let mut m = TreeModel::default();
        let all = fude_fs::WalkedFiles { tracked: vec![rel("src/plan.rs")], ignored: vec![rel("docs/plan.md")] };
        m.set_filter("plan", Some(&all));
        let marks: Vec<(String, bool)> = m.rows(None).into_iter().map(|r| (r.rel.as_str().to_string(), r.ignored)).collect();
        assert_eq!(
            marks,
            vec![("docs".into(), true), ("docs/plan.md".into(), true), ("src".into(), false), ("src/plan.rs".into(), false)]
        );
    }

    #[test]
    fn invalidate_all_marks_loaded_dirs() {
        let mut m = TreeModel::default();
        m.set_children(&RelPath::root(), vec![entry("src", true)]);
        m.set_children(&rel("src"), vec![entry("a.rs", false)]);
        m.set_filter("", Some(&fude_fs::WalkedFiles::default()));
        assert_eq!(m.invalidate_all(), vec![RelPath::root(), rel("src")]);
        assert!(m.needs_load(&RelPath::root()) && m.needs_load(&rel("src")));
        assert!(!m.has_file_list());
    }

    #[test]
    fn unloaded_expanded_dirs_are_listed_shallow_first() {
        let mut m = TreeModel::default();
        m.reveal(&rel("a/b"));
        assert_eq!(m.unloaded_expanded(), vec![rel("a"), rel("a/b")]);
        m.set_children(&rel("a"), vec![entry("b", true)]);
        assert_eq!(m.unloaded_expanded(), vec![rel("a/b")]);
    }
}
