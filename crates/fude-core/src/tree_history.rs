//! ツリーのファイル操作の取り消しとやり直しの履歴。札（ゴミ箱から戻す手がかり）は型引数にする
//! （fude-core は fude-fs に依存しないため）。

/// 1 プロジェクトで持つ取り消しの数。超えたら古いものから捨てる。
pub const HISTORY_MAX: usize = 100;

/// 履歴の 1 件。パスはプロジェクトのルートからの相対パス。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeOp<T> {
    /// 新規ファイル／新規フォルダ。取り消してゴミ箱の中にあるときは trashed に札を持つ。
    Created { path: String, trashed: Option<T> },
    Renamed { from: String, to: String },
    /// 削除（ゴミ箱に送った）。
    Deleted { path: String, ticket: T },
}

/// 取り消しとやり直しの履歴。取り出した op は、成功なら done_*、止めたなら restore_* で戻し、
/// 失敗して捨てるなら何もしない。
#[derive(Debug)]
pub struct TreeHistory<T> {
    undo: Vec<TreeOp<T>>,
    redo: Vec<TreeOp<T>>,
}

impl<T> Default for TreeHistory<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TreeHistory<T> {
    pub fn new() -> Self {
        Self { undo: Vec::new(), redo: Vec::new() }
    }

    /// 新しい操作を積む。やり直しの履歴は捨てる。
    pub fn push(&mut self, op: TreeOp<T>) {
        self.redo.clear();
        self.push_undo(op);
    }

    fn push_undo(&mut self, op: TreeOp<T>) {
        self.undo.push(op);
        if self.undo.len() > HISTORY_MAX {
            self.undo.remove(0);
        }
    }

    pub fn take_undo(&mut self) -> Option<TreeOp<T>> {
        self.undo.pop()
    }

    pub fn take_redo(&mut self) -> Option<TreeOp<T>> {
        self.redo.pop()
    }

    /// 取り消しが成功した結果を、やり直しへ積む。
    pub fn done_undo(&mut self, op: TreeOp<T>) {
        self.redo.push(op);
    }

    /// やり直しが成功した結果を、取り消しへ積む（やり直しの残りは捨てない）。
    pub fn done_redo(&mut self, op: TreeOp<T>) {
        self.push_undo(op);
    }

    /// 取り消しを止めたので、元の位置へ戻す。
    pub fn restore_undo(&mut self, op: TreeOp<T>) {
        self.undo.push(op);
    }

    /// やり直しを止めたので、元の位置へ戻す。
    pub fn restore_redo(&mut self, op: TreeOp<T>) {
        self.redo.push(op);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

/// 名前の入力欄で最初に選ぶ長さ（バイト数）。ファイルは最後の `.` の手前まで。先頭の `.` しか無い名前、
/// `.` の無い名前、フォルダは全部。
pub fn name_stem_len(name: &str, is_dir: bool) -> usize {
    if is_dir {
        return name.len();
    }
    match name.rfind('.') {
        Some(i) if i > 0 => i,
        _ => name.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_clears_redo_and_caps() {
        let mut h = TreeHistory::<u32>::new();
        h.push(TreeOp::Renamed { from: "a".into(), to: "b".into() });
        let op = h.take_undo().unwrap();
        h.done_undo(op);
        assert!(h.can_redo());
        h.push(TreeOp::Created { path: "c".into(), trashed: None });
        assert!(!h.can_redo());
        for i in 0..150 {
            h.push(TreeOp::Deleted { path: format!("{i}"), ticket: i });
        }
        let mut n = 0;
        while h.take_undo().is_some() {
            n += 1;
        }
        assert_eq!(n, HISTORY_MAX);
    }

    #[test]
    fn undo_redo_order_and_failure_drops_one() {
        let mut h = TreeHistory::<u32>::new();
        h.push(TreeOp::Deleted { path: "x".into(), ticket: 1 });
        h.push(TreeOp::Deleted { path: "x".into(), ticket: 2 });
        // 新しい方から取り出す。
        assert_eq!(h.take_undo(), Some(TreeOp::Deleted { path: "x".into(), ticket: 2 }));
        // 失敗したので何も戻さない → 前の 1 件は残る。
        assert_eq!(h.take_undo(), Some(TreeOp::Deleted { path: "x".into(), ticket: 1 }));
        assert!(!h.can_redo());
    }

    #[test]
    fn redo_keeps_rest_of_redo() {
        let mut h = TreeHistory::<u32>::new();
        h.push(TreeOp::Renamed { from: "a".into(), to: "b".into() });
        h.push(TreeOp::Renamed { from: "b".into(), to: "c".into() });
        let op = h.take_undo().unwrap();
        h.done_undo(op);
        let op = h.take_undo().unwrap();
        h.done_undo(op);
        let op = h.take_redo().unwrap();
        assert_eq!(op, TreeOp::Renamed { from: "a".into(), to: "b".into() });
        h.done_redo(op);
        assert!(h.can_redo());
        assert!(h.can_undo());
    }

    #[test]
    fn blocked_op_goes_back() {
        let mut h = TreeHistory::<u32>::new();
        h.push(TreeOp::Created { path: "n.md".into(), trashed: None });
        let op = h.take_undo().unwrap();
        h.restore_undo(op);
        assert!(h.can_undo());
        let op = h.take_undo().unwrap();
        h.done_undo(op);
        let op = h.take_redo().unwrap();
        h.restore_redo(op);
        assert!(h.can_redo());
    }

    #[test]
    fn stem_len() {
        assert_eq!(name_stem_len("b.md", false), 1);
        assert_eq!(name_stem_len("a.tar.gz", false), 5);
        assert_eq!(name_stem_len(".gitignore", false), 10);
        assert_eq!(name_stem_len("Makefile", false), 8);
        assert_eq!(name_stem_len("docs.v2", true), 7);
        assert_eq!(name_stem_len("日本.md", false), "日本".len());
    }
}
