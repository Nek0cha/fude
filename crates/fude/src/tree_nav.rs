//! ツリーの選択をキーで動かす規則（計画8c）。描画した行の並びの上で決める。GUI には依らない。
//! 並びに無い選択は、無いものとして扱う（開いた直後はフォルダを裏で読んでいる途中のことがあるため）。

use fude_fs::RelPath;

use crate::tree_model::{RowSlot, TreeRow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// ← と → の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Horizontal {
    Expand(RelPath),
    Collapse(RelPath),
    Select(RelPath),
    Nothing,
}

/// 名前の入力欄を除いた行と、その RowSlot での位置。
fn entries(rows: &[RowSlot]) -> impl DoubleEndedIterator<Item = (usize, &TreeRow)> {
    rows.iter().enumerate().filter_map(|(i, s)| match s {
        RowSlot::Entry(row) => Some((i, row)),
        RowSlot::Edit { .. } => None,
    })
}

/// 選択の行と、その RowSlot での位置。並びに無ければ None。
pub(crate) fn selected_row<'a>(rows: &'a [RowSlot], selected: Option<&RelPath>) -> Option<(usize, &'a TreeRow)> {
    let selected = selected?;
    entries(rows).find(|(_, row)| row.rel == *selected)
}

/// key で動かした先の行。並びに無い選択から、下向き（↓、Home、PageDown）は先頭へ、上向きは末尾へ動く。
pub(crate) fn move_selection(rows: &[RowSlot], selected: Option<&RelPath>, key: NavKey, page: usize) -> Option<RelPath> {
    let list: Vec<&TreeRow> = entries(rows).map(|(_, row)| row).collect();
    let last = list.len().checked_sub(1)?;
    let current = selected.and_then(|s| list.iter().position(|row| row.rel == *s));
    let page = page.max(1);
    let to = match (key, current) {
        (NavKey::Home, _) | (NavKey::Down | NavKey::PageDown, None) => 0,
        (NavKey::End, _) | (NavKey::Up | NavKey::PageUp, None) => last,
        (NavKey::Down, Some(i)) => (i + 1).min(last),
        (NavKey::Up, Some(i)) => i.saturating_sub(1),
        (NavKey::PageDown, Some(i)) => (i + page).min(last),
        (NavKey::PageUp, Some(i)) => i.saturating_sub(page),
    };
    Some(list[to].rel.clone())
}

/// → は閉じたフォルダを開き、開いたフォルダでは最初の子へ移る。← は開いたフォルダを閉じ、それ以外は親へ移る。
/// can_toggle が false なら開閉はしない（子や親への移動は残す）。
pub(crate) fn horizontal(rows: &[RowSlot], selected: Option<&RelPath>, right: bool, can_toggle: bool) -> Horizontal {
    let Some((ix, row)) = selected_row(rows, selected) else { return Horizontal::Nothing };
    if right {
        if !row.is_dir {
            return Horizontal::Nothing;
        }
        if !row.expanded {
            return if can_toggle { Horizontal::Expand(row.rel.clone()) } else { Horizontal::Nothing };
        }
        return match rows.get(ix + 1) {
            Some(RowSlot::Entry(child)) if child.depth == row.depth + 1 => Horizontal::Select(child.rel.clone()),
            _ => Horizontal::Nothing,
        };
    }
    if row.is_dir && row.expanded {
        return if can_toggle { Horizontal::Collapse(row.rel.clone()) } else { Horizontal::Nothing };
    }
    let Some((parent, _)) = row.rel.as_str().rsplit_once('/') else { return Horizontal::Nothing };
    match entries(rows).find(|(_, r)| r.rel.as_str() == parent) {
        Some((_, r)) => Horizontal::Select(r.rel.clone()),
        None => Horizontal::Nothing,
    }
}

/// removed を消したあとに選ぶ行。後ろで removed とその中ではない最初の行、無ければ前の行。
pub(crate) fn after_removal(rows: &[RowSlot], removed: &RelPath) -> Option<RelPath> {
    let inside = |row: &TreeRow| {
        row.rel == *removed || row.rel.as_str().strip_prefix(removed.as_str()).is_some_and(|rest| rest.starts_with('/'))
    };
    let list: Vec<&TreeRow> = entries(rows).map(|(_, row)| row).collect();
    let at = list.iter().position(|row| row.rel == *removed)?;
    let next = list[at + 1..].iter().find(|row| !inside(row));
    let prev = || list[..at].iter().rev().find(|row| !inside(row));
    next.or_else(prev).map(|row| row.rel.clone())
}

/// 削除が済んだときに選ぶ行。消した行がまだ並びにあれば今の並びで決め、監視の読み直しで先に消えていれば
/// 確認の前に決めておいた planned にする。
pub(crate) fn next_after_delete(rows: &[RowSlot], removed: &RelPath, planned: Option<RelPath>) -> Option<RelPath> {
    if selected_row(rows, Some(removed)).is_some() { after_removal(rows, removed) } else { planned }
}

/// 並びの先頭のファイルの行（フォルダは飛ばす）。
pub(crate) fn first_file(rows: &[RowSlot]) -> Option<RelPath> {
    entries(rows).find(|(_, row)| !row.is_dir).map(|(_, row)| row.rel.clone())
}

/// PageUp / PageDown で動かす行数。見えている行数から 1 を引く（最低 1）。高さが未計測なら 10。
pub(crate) fn page_rows(list_height: f32, row_height: f32) -> usize {
    if list_height <= 0. || row_height <= 0. {
        return 10;
    }
    ((list_height / row_height).floor() as usize).saturating_sub(1).max(1)
}

#[cfg(test)]
mod tests {
    use super::{Horizontal, NavKey, after_removal, first_file, horizontal, move_selection, next_after_delete, page_rows};
    use crate::tree_model::{RowSlot, TreeRow};
    use fude_fs::RelPath;

    fn rel(p: &str) -> RelPath {
        RelPath::new(p).unwrap()
    }

    fn entry(path: &str, depth: usize, is_dir: bool, expanded: bool) -> RowSlot {
        let name = path.rsplit('/').next().unwrap().to_string();
        RowSlot::Entry(TreeRow { rel: rel(path), name, depth, is_dir, expanded, mark: None })
    }

    /// d（開いている）、d/x、d/y、e（閉じている）、a.txt、b.txt。
    fn sample() -> Vec<RowSlot> {
        vec![
            entry("d", 0, true, true),
            entry("d/x", 1, false, false),
            entry("d/y", 1, false, false),
            entry("e", 0, true, false),
            entry("a.txt", 0, false, false),
            entry("b.txt", 0, false, false),
        ]
    }

    fn mv(rows: &[RowSlot], from: Option<&str>, key: NavKey, page: usize) -> Option<RelPath> {
        move_selection(rows, from.map(rel).as_ref(), key, page)
    }

    #[test]
    fn down_moves_and_stops_at_end() {
        let rows = sample();
        assert_eq!(mv(&rows, None, NavKey::Down, 1), Some(rel("d")));
        assert_eq!(mv(&rows, Some("d"), NavKey::Down, 1), Some(rel("d/x")));
        assert_eq!(mv(&rows, Some("b.txt"), NavKey::Down, 1), Some(rel("b.txt")));
    }

    #[test]
    fn up_moves_and_stops_at_top() {
        let rows = sample();
        assert_eq!(mv(&rows, Some("d/x"), NavKey::Up, 1), Some(rel("d")));
        assert_eq!(mv(&rows, Some("d"), NavKey::Up, 1), Some(rel("d")));
        assert_eq!(mv(&rows, None, NavKey::Up, 1), Some(rel("b.txt")));
    }

    #[test]
    fn home_end_and_pages() {
        let rows = sample();
        assert_eq!(mv(&rows, Some("a.txt"), NavKey::Home, 3), Some(rel("d")));
        assert_eq!(mv(&rows, Some("d"), NavKey::End, 3), Some(rel("b.txt")));
        assert_eq!(mv(&rows, Some("d"), NavKey::PageDown, 3), Some(rel("e")));
        assert_eq!(mv(&rows, Some("b.txt"), NavKey::PageUp, 3), Some(rel("d/y")));
        assert_eq!(mv(&rows, Some("d"), NavKey::PageDown, 100), Some(rel("b.txt")));
    }

    #[test]
    fn missing_selection_counts_as_none() {
        let rows = sample();
        assert_eq!(mv(&rows, Some("gone"), NavKey::Down, 1), Some(rel("d")));
        assert_eq!(mv(&rows, Some("gone"), NavKey::Up, 1), Some(rel("b.txt")));
    }

    #[test]
    fn edit_slots_are_skipped() {
        let rows = vec![
            entry("d", 0, true, true),
            RowSlot::Edit { depth: 1, kind: crate::tree_menu::EditKind::NewFile },
            entry("a.txt", 0, false, false),
        ];
        assert_eq!(mv(&rows, Some("d"), NavKey::Down, 1), Some(rel("a.txt")));
    }

    #[test]
    fn empty_rows_give_none() {
        assert_eq!(mv(&[], None, NavKey::Down, 1), None);
    }

    fn hz(rows: &[RowSlot], at: &str, right: bool, can_toggle: bool) -> Horizontal {
        horizontal(rows, Some(&rel(at)), right, can_toggle)
    }

    #[test]
    fn right_expands_or_enters() {
        let rows = sample();
        assert_eq!(hz(&rows, "e", true, true), Horizontal::Expand(rel("e")));
        assert_eq!(hz(&rows, "d", true, true), Horizontal::Select(rel("d/x")));
        assert_eq!(hz(&rows, "a.txt", true, true), Horizontal::Nothing);
    }

    #[test]
    fn left_collapses_or_goes_up() {
        let rows = sample();
        assert_eq!(hz(&rows, "d", false, true), Horizontal::Collapse(rel("d")));
        assert_eq!(hz(&rows, "d/y", false, true), Horizontal::Select(rel("d")));
        assert_eq!(hz(&rows, "a.txt", false, true), Horizontal::Nothing);
        assert_eq!(hz(&rows, "e", false, true), Horizontal::Nothing);
    }

    #[test]
    fn horizontal_does_nothing_without_toggle() {
        let rows = sample();
        assert_eq!(hz(&rows, "e", true, false), Horizontal::Nothing);
        assert_eq!(hz(&rows, "d", false, false), Horizontal::Nothing);
        assert_eq!(hz(&rows, "d/y", false, false), Horizontal::Select(rel("d")));
    }

    #[test]
    fn after_removal_picks_next_or_previous() {
        let rows = sample();
        assert_eq!(after_removal(&rows, &rel("a.txt")), Some(rel("b.txt")));
        assert_eq!(after_removal(&rows, &rel("b.txt")), Some(rel("a.txt")));
        assert_eq!(after_removal(&[entry("a.txt", 0, false, false)], &rel("a.txt")), None);
    }

    #[test]
    fn next_after_delete_falls_back_to_planned() {
        let rows = sample();
        // 消した行がまだ並びにあれば、今の並びで決める。
        assert_eq!(next_after_delete(&rows, &rel("a.txt"), Some(rel("e"))), Some(rel("b.txt")));
        // 監視の読み直しで先に消えていたら、確認の前に決めておいた行にする。
        assert_eq!(next_after_delete(&rows, &rel("gone"), Some(rel("e"))), Some(rel("e")));
        assert_eq!(next_after_delete(&rows, &rel("gone"), None), None);
    }

    #[test]
    fn after_removal_skips_descendants() {
        assert_eq!(after_removal(&sample(), &rel("d")), Some(rel("e")));
    }

    #[test]
    fn first_file_skips_folders() {
        assert_eq!(first_file(&sample()), Some(rel("d/x")));
        assert_eq!(first_file(&[entry("d", 0, true, false)]), None);
    }

    #[test]
    fn page_rows_from_height() {
        assert_eq!(page_rows(240., 24.), 9);
        assert_eq!(page_rows(10., 24.), 1);
        assert_eq!(page_rows(0., 24.), 10);
    }
}
