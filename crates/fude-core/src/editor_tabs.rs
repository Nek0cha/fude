//! エディタ列のタブの並び（設計書 §4.1.3）。開く、閉じる、選ぶ、名前の変更の規則を持つ。

use serde::{Deserialize, Serialize};

use crate::panel::{FileMode, renamed_rel};

/// エディタ列の 1 枚のタブ。今はファイルだけを扱う。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorItem {
    /// プロジェクトのルートからの相対パス。
    pub rel: String,
    pub mode: FileMode,
}

/// エディタ列のタブの並びと、選択中の位置。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorTabs {
    items: Vec<EditorItem>,
    active: Option<usize>,
}

/// `EditorTabs::open` の結果。どちらも選んだタブの位置を持つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    New(usize),
    Existing(usize),
}

impl EditorTabs {
    /// 同じ相対パスのタブがあればそれを選ぶだけにする。無ければ選択中のタブの右に足して選ぶ。
    pub fn open(&mut self, item: EditorItem) -> Opened {
        if let Some(i) = self.position(&item.rel) {
            self.active = Some(i);
            return Opened::Existing(i);
        }
        let at = self.active.map_or(self.items.len(), |a| a + 1);
        self.items.insert(at, item);
        self.active = Some(at);
        Opened::New(at)
    }

    /// タブを閉じる。選択中を閉じたら右の隣、無ければ左の隣を選ぶ。
    pub fn close(&mut self, index: usize) -> Option<EditorItem> {
        if index >= self.items.len() {
            return None;
        }
        let item = self.items.remove(index);
        self.active = match self.active {
            _ if self.items.is_empty() => None,
            Some(a) if a == index => Some(index.min(self.items.len() - 1)),
            Some(a) if a > index => Some(a - 1),
            other => other,
        };
        Some(item)
    }

    pub fn select(&mut self, index: usize) {
        if index < self.items.len() {
            self.active = Some(index);
        }
    }

    /// 選択を delta だけ動かす。端を越えたら反対の端へ回る。
    pub fn cycle(&mut self, delta: i32) {
        let n = self.items.len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.active.unwrap_or(0) as i32;
        self.active = Some((cur + delta).rem_euclid(n) as usize);
    }

    pub fn active(&self) -> Option<&EditorItem> {
        self.active.and_then(|i| self.items.get(i))
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn items(&self) -> &[EditorItem] {
        &self.items
    }

    pub fn position(&self, rel: &str) -> Option<usize> {
        self.items.iter().position(|i| i.rel == rel)
    }

    /// ファイルかフォルダの名前の変更に、タブの相対パスを付いていかせる。
    pub fn rename(&mut self, from: &str, to: &str) {
        for item in &mut self.items {
            if let Some(rel) = renamed_rel(&item.rel, from, to) {
                item.rel = rel;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn set_mode(&mut self, index: usize, mode: FileMode) {
        if let Some(item) = self.items.get_mut(index) {
            item.mode = mode;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(rel: &str) -> EditorItem {
        EditorItem { rel: rel.into(), mode: FileMode::Full }
    }

    #[test]
    fn open_inserts_right_of_active_and_selects() {
        let mut t = EditorTabs::default();
        assert_eq!(t.open(it("a")), Opened::New(0));
        assert_eq!(t.open(it("b")), Opened::New(1));
        t.select(0);
        assert_eq!(t.open(it("c")), Opened::New(1));
        let rels: Vec<&str> = t.items().iter().map(|i| i.rel.as_str()).collect();
        assert_eq!(rels, ["a", "c", "b"]);
        assert_eq!(t.active().unwrap().rel, "c");
    }

    #[test]
    fn open_existing_selects_without_adding() {
        let mut t = EditorTabs::default();
        t.open(it("a"));
        t.open(it("b"));
        assert_eq!(t.open(EditorItem { rel: "a".into(), mode: FileMode::Diff }), Opened::Existing(0));
        assert_eq!(t.items().len(), 2);
        assert_eq!(t.active_index(), Some(0));
        // 既存のタブの mode は変えない（持ち主が mode を指定したときだけ set_mode で変える）。
        assert_eq!(t.items()[0].mode, FileMode::Full);
    }

    #[test]
    fn close_selects_right_then_left() {
        let mut t = EditorTabs::default();
        for r in ["a", "b", "c"] {
            t.open(it(r));
        }
        t.select(1);
        assert_eq!(t.close(1).unwrap().rel, "b");
        assert_eq!(t.active().unwrap().rel, "c");
        t.close(1);
        assert_eq!(t.active().unwrap().rel, "a");
        t.close(0);
        assert!(t.is_empty());
        assert_eq!(t.active_index(), None);
    }

    #[test]
    fn closing_inactive_keeps_active_item() {
        let mut t = EditorTabs::default();
        for r in ["a", "b", "c"] {
            t.open(it(r));
        }
        t.select(2);
        t.close(0);
        assert_eq!(t.active().unwrap().rel, "c");
    }

    #[test]
    fn cycle_wraps() {
        let mut t = EditorTabs::default();
        for r in ["a", "b"] {
            t.open(it(r));
        }
        t.cycle(1);
        assert_eq!(t.active().unwrap().rel, "a");
        t.cycle(-1);
        assert_eq!(t.active().unwrap().rel, "b");
    }

    #[test]
    fn rename_updates_folder_children() {
        let mut t = EditorTabs::default();
        for r in ["src/x/a.rs", "src2/b.rs", "c.rs"] {
            t.open(it(r));
        }
        t.rename("src", "lib");
        let rels: Vec<&str> = t.items().iter().map(|i| i.rel.as_str()).collect();
        assert_eq!(rels, ["lib/x/a.rs", "src2/b.rs", "c.rs"]);
        assert_eq!(t.position("lib/x/a.rs"), Some(0));
    }
}
