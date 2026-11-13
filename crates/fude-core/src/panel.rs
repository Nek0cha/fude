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
    /// 今の項目より先を捨てて足す。今の項目と同じファイルなら mode だけを変えて false を返す。
    pub fn open(&mut self, item: PanelItem) -> bool {
        if let Some(cur) = self.items.get_mut(self.index)
            && cur.rel == item.rel
        {
            cur.mode = item.mode;
            return false;
        }
        if !self.items.is_empty() {
            self.items.truncate(self.index + 1);
        }
        self.items.push(item);
        self.index = self.items.len() - 1;
        true
    }

    /// from（ファイルかフォルダ）の名前が to に変わった。履歴の項目のパスを合わせる。
    pub fn rename(&mut self, from: &str, to: &str) {
        for item in &mut self.items {
            if let Some(rel) = renamed_rel(&item.rel, from, to) {
                item.rel = rel;
            }
        }
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
    /// 何もしない（中身が同じ、または一時的に読めなかった。次の通知を待つ）。
    Ignore,
}

/// 外部での変更の通知を受けて、ファイルを読み直した結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    /// 最後に読んだ（保存した）中身と同じ。
    Same,
    /// 中身が変わった。
    Changed,
    /// ファイルが無い。
    Missing,
    /// 読めなかった（ほかのプロセスが書き込み中の共有違反など）。
    Failed,
}

pub fn external_change(dirty: bool, outcome: ReadOutcome) -> ExternalAction {
    match (dirty, outcome) {
        (_, ReadOutcome::Same | ReadOutcome::Failed) => ExternalAction::Ignore,
        (_, ReadOutcome::Missing) => ExternalAction::MarkMissing,
        (false, ReadOutcome::Changed) => ExternalAction::Reload,
        (true, ReadOutcome::Changed) => ExternalAction::AskKeepOrReload,
    }
}

/// rel が from（ファイルかフォルダ）の名前の変更の影響を受けるなら、新しいパスを返す。
pub fn renamed_rel(rel: &str, from: &str, to: &str) -> Option<String> {
    if rel == from {
        return Some(to.to_string());
    }
    let rest = rel.strip_prefix(from)?.strip_prefix('/')?;
    Some(format!("{to}/{rest}"))
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
        assert_eq!(external_change(false, ReadOutcome::Changed), ExternalAction::Reload);
        assert_eq!(external_change(true, ReadOutcome::Changed), ExternalAction::AskKeepOrReload);
        assert_eq!(external_change(false, ReadOutcome::Missing), ExternalAction::MarkMissing);
        assert_eq!(external_change(true, ReadOutcome::Missing), ExternalAction::MarkMissing);
    }

    #[test]
    fn transient_read_error_keeps_editor() {
        // 書き込み中の共有違反などで読めなかったときは、エディタを壊さず次の通知を待つ。
        assert_eq!(external_change(false, ReadOutcome::Failed), ExternalAction::Ignore);
        assert_eq!(external_change(true, ReadOutcome::Failed), ExternalAction::Ignore);
        assert_eq!(external_change(false, ReadOutcome::Same), ExternalAction::Ignore);
    }

    #[test]
    fn open_reports_same_file() {
        // 同じファイルを開き直したかどうか。同じなら、持ち主は今の表示（未保存の変更を含む）を使い続ける。
        let mut h = PanelHistory::default();
        assert!(h.open(item("a", FileMode::Full)));
        assert!(!h.open(item("a", FileMode::Diff)));
        assert!(h.open(item("b", FileMode::Full)));
    }

    #[test]
    fn renamed_rel_follows_file_and_folder() {
        assert_eq!(renamed_rel("src/a.rs", "src/a.rs", "src/b.rs").as_deref(), Some("src/b.rs"));
        assert_eq!(renamed_rel("src/x/a.rs", "src", "lib").as_deref(), Some("lib/x/a.rs"));
        // 名前の先頭が同じだけの別のファイルは変えない。
        assert_eq!(renamed_rel("src2/a.rs", "src", "lib"), None);
        assert_eq!(renamed_rel("other.rs", "src/a.rs", "src/b.rs"), None);
    }

    #[test]
    fn history_rename_updates_items() {
        let mut h = PanelHistory::default();
        h.open(item("src/a.rs", FileMode::Full));
        h.open(item("b.rs", FileMode::Full));
        h.rename("src", "lib");
        assert_eq!(h.back().unwrap().rel, "lib/a.rs");
        assert_eq!(h.forward().unwrap().rel, "b.rs");
    }
}
