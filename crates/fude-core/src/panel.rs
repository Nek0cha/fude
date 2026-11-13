//! 右パネルの履歴と、外部での変更への対応の規則（設計書 §4.1 の右パネル、§5.3）。

/// ファイルの表示の仕方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    /// HEAD との差分（閲覧専用）。
    Diff,
    /// ファイル全体（編集できる）。
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelItem {
    /// プロジェクトのルートからの相対パス。
    pub rel: String,
    pub mode: FileMode,
}

/// 右パネルの表示の履歴。戻る／進むでたどる。
#[derive(Clone, Debug, Default)]
pub struct PanelHistory {
    items: Vec<PanelItem>,
    index: usize,
}

impl PanelHistory {
    /// 今の項目より先を捨てて足す。今の項目と同じファイルなら mode だけを変える。
    pub fn open(&mut self, item: PanelItem) {
        if let Some(cur) = self.items.get_mut(self.index)
            && cur.rel == item.rel
        {
            cur.mode = item.mode;
            return;
        }
        if !self.items.is_empty() {
            self.items.truncate(self.index + 1);
        }
        self.items.push(item);
        self.index = self.items.len() - 1;
    }

    pub fn current(&self) -> Option<&PanelItem> {
        self.items.get(self.index)
    }

    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index + 1 < self.items.len()
    }

    pub fn back(&mut self) -> Option<&PanelItem> {
        if !self.can_back() {
            return None;
        }
        self.index -= 1;
        self.current()
    }

    pub fn forward(&mut self) -> Option<&PanelItem> {
        if !self.can_forward() {
            return None;
        }
        self.index += 1;
        self.current()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.index = 0;
    }
}

/// 開いているファイルが外部で変わったときにすること。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalAction {
    /// 黙って読み直す（カーソルとスクロールは保つ）。
    Reload,
    /// 上部の帯で、読み直すか、このままにするかを選ばせる。
    AskKeepOrReload,
    /// ファイルが無くなった。帯で知らせ、中身は残す（保存すると作り直す）。
    MarkMissing,
}

pub fn external_change(dirty: bool, exists: bool) -> ExternalAction {
    match (dirty, exists) {
        (_, false) => ExternalAction::MarkMissing,
        (false, true) => ExternalAction::Reload,
        (true, true) => ExternalAction::AskKeepOrReload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(rel: &str, mode: FileMode) -> PanelItem {
        PanelItem { rel: rel.into(), mode }
    }

    #[test]
    fn history_back_forward_and_truncate() {
        let mut h = PanelHistory::default();
        assert!(h.current().is_none());
        h.open(item("a", FileMode::Full));
        h.open(item("b", FileMode::Diff));
        h.open(item("c", FileMode::Full));
        assert_eq!(h.back().unwrap().rel, "b");
        assert!(h.can_forward());
        h.open(item("d", FileMode::Full));
        assert!(!h.can_forward());
        assert_eq!(h.back().unwrap().rel, "b");
        assert_eq!(h.back().unwrap().rel, "a");
        assert!(!h.can_back());
        assert!(h.back().is_none());
        assert_eq!(h.current().unwrap().rel, "a");
        assert_eq!(h.forward().unwrap().rel, "b");
        assert_eq!(h.forward().unwrap().rel, "d");
    }

    #[test]
    fn reopening_current_only_changes_mode() {
        let mut h = PanelHistory::default();
        h.open(item("a", FileMode::Diff));
        h.open(item("a", FileMode::Full));
        assert!(!h.can_back());
        assert_eq!(h.current().unwrap().mode, FileMode::Full);
        h.clear();
        assert!(h.current().is_none());
    }

    #[test]
    fn external_change_rules() {
        assert_eq!(external_change(false, true), ExternalAction::Reload);
        assert_eq!(external_change(true, true), ExternalAction::AskKeepOrReload);
        assert_eq!(external_change(false, false), ExternalAction::MarkMissing);
        assert_eq!(external_change(true, false), ExternalAction::MarkMissing);
    }
}
