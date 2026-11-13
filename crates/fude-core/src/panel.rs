//! ファイルの表示の仕方と、外部での変更への対応の規則（設計書 §5.3）。

/// ファイルの表示の仕方。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FileMode {
    /// HEAD との差分（閲覧専用）。
    Diff,
    /// ファイル全体（編集できる）。
    Full,
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
    fn renamed_rel_follows_file_and_folder() {
        assert_eq!(renamed_rel("src/a.rs", "src/a.rs", "src/b.rs").as_deref(), Some("src/b.rs"));
        assert_eq!(renamed_rel("src/x/a.rs", "src", "lib").as_deref(), Some("lib/x/a.rs"));
        // 名前の先頭が同じだけの別のファイルは変えない。
        assert_eq!(renamed_rel("src2/a.rs", "src", "lib"), None);
        assert_eq!(renamed_rel("other.rs", "src/a.rs", "src/b.rs"), None);
    }

}
