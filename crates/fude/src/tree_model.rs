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
    all_files: Option<Vec<RelPath>>,
    changed_only: bool,
}

/// パスの集まりから作る木（絞り込みと「変更だけ」で使う）。
#[derive(Default)]
struct PathTree {
    dirs: BTreeMap<String, PathTree>,
    files: Vec<String>,
}

impl PathTree {
    fn insert(&mut self, parts: &[&str]) {
        match parts {
            [] => {}
            [file] => self.files.push(file.to_string()),
            [dir, rest @ ..] => self.dirs.entry(dir.to_string()).or_default().insert(rest),
        }
    }

    fn rows(&self, prefix: &str, depth: usize, status: Option<&GitStatus>, out: &mut Vec<TreeRow>) {
        for (name, sub) in &self.dirs {
            let rel = join(prefix, name);
            out.push(row(&rel, name, depth, true, true, status));
            sub.rows(&rel, depth + 1, status, out);
        }
        let mut files = self.files.clone();
        files.sort();
        files.dedup();
        for name in files {
            let rel = join(prefix, &name);
            out.push(row(&rel, &name, depth, false, false, status));
        }
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") }
}

fn row(rel: &str, name: &str, depth: usize, is_dir: bool, expanded: bool, status: Option<&GitStatus>) -> TreeRow {
    let rel = RelPath::new(rel).unwrap_or_else(|_| RelPath::root());
    let mark = if is_dir { None } else { status.and_then(|s| s.mark(&rel)) };
    TreeRow { rel, name: name.to_string(), depth, is_dir, expanded, mark }
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

    /// ファイル名で絞り込む（大文字小文字を区別しない部分一致）。all_files は全ファイルの一覧（無ければ、届くまで何も出さない）。
    pub fn set_filter(&mut self, query: &str, all_files: Option<&[RelPath]>) {
        self.filter = query.trim().to_lowercase();
        if let Some(all) = all_files {
            self.all_files = Some(all.to_vec());
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

    pub fn has_file_list(&self) -> bool {
        self.all_files.is_some()
    }

    pub fn rows(&self, status: Option<&GitStatus>) -> Vec<TreeRow> {
        let mut out = Vec::new();
        if self.filtering() {
            let Some(all) = &self.all_files else { return out };
            let mut tree = PathTree::default();
            for f in all {
                let name = f.as_str().rsplit('/').next().unwrap_or_default();
                if name.to_lowercase().contains(&self.filter) {
                    tree.insert(&f.as_str().split('/').collect::<Vec<_>>());
                }
            }
            tree.rows("", 0, status, &mut out);
            return out;
        }
        if self.changed_only {
            let mut tree = PathTree::default();
            for (p, _) in status.map(|s| s.entries.as_slice()).unwrap_or_default() {
                tree.insert(&p.as_str().split('/').collect::<Vec<_>>());
            }
            tree.rows("", 0, status, &mut out);
            return out;
        }
        self.dir_rows("", 0, status, &mut out);
        out
    }

    fn dir_rows(&self, dir: &str, depth: usize, status: Option<&GitStatus>, out: &mut Vec<TreeRow>) {
        let Some(entries) = self.children.get(dir) else { return };
        for e in entries {
            let rel = join(dir, &e.name);
            let expanded = e.is_dir && self.expanded.contains(&rel);
            out.push(row(&rel, &e.name, depth, e.is_dir, expanded, status));
            if expanded {
                self.dir_rows(&rel, depth + 1, status, out);
            }
        }
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
        DirEntry { name: name.into(), is_dir }
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
        let all = vec![rel("README.md"), rel("src/main.rs"), rel("src/ui/Button.tsx")];
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
        TreeRow { name: p.rsplit('/').next().unwrap().into(), rel, depth, is_dir, expanded: is_dir, mark: None }
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
}
