//! パレットのあいまい検索（設計書 §5.9）。順位付けは nucleo-matcher（helix のもの）に任せる。

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// 1 件の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ranked {
    pub index: usize,
    pub score: u32,
    /// 一致した文字（char）の位置。昇順。
    pub positions: Vec<u32>,
}

/// query で candidates をあいまいに絞り、点の高い順（同点は index の順）に最大 limit 件。
/// 空の query は先頭から limit 件（score 0、positions 空）。path_mode はパス向けの設定（区切りを重く見る）。
pub fn rank(query: &str, candidates: &[&str], path_mode: bool, limit: usize) -> Vec<Ranked> {
    if query.trim().is_empty() {
        return (0..candidates.len().min(limit)).map(|index| Ranked { index, score: 0, positions: Vec::new() }).collect();
    }
    let config = if path_mode { Config::DEFAULT.match_paths() } else { Config::DEFAULT };
    let mut matcher = Matcher::new(config);
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut ranked = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let mut positions = Vec::new();
        let Some(score) = pattern.indices(Utf32Str::new(candidate, &mut buf), &mut matcher, &mut positions) else {
            continue;
        };
        positions.sort_unstable();
        positions.dedup();
        ranked.push(Ranked { index, score, positions });
    }
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    ranked.truncate(limit);
    ranked
}

#[cfg(test)]
mod tests {
    use super::rank;

    #[test]
    fn ranks_path_matches() {
        let files = ["src/main.rs", "crates/fude/src/main_area.rs", "README.md", "docs/manual.md"];
        let r = rank("mainrs", &files, true, 10);
        assert_eq!(r[0].index, 0);
        assert!(r.iter().all(|x| x.index != 2));
        assert_eq!(rank("", &files, true, 2).iter().map(|x| x.index).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn positions_point_at_chars() {
        let r = rank("ab", &["xaxb"], false, 10);
        assert_eq!(r[0].positions, [1, 3]);
        // 日本語も文字の位置で返す。
        let r = rank("設計", &["docs/設計書.md"], true, 10);
        assert_eq!(r[0].positions, [5, 6]);
    }

    #[test]
    fn limit_applies_after_ranking() {
        let r = rank("a", &["b", "a", "ba", "ab"], false, 2);
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.index != 0));
    }
}
