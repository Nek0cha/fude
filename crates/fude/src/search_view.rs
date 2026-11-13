//! 全体検索の結果のタブ（設計書 §5.6）。ファイルごとにまとめて出し、行を押すとそのファイルのその行を開く。
//! 検索は ProjectFs::search で裏で行う（WSL のプロジェクトは fude serve の中で走る）。

use std::collections::HashSet;
use std::sync::Arc;

use fude_fs::ProjectFs;
use fude_fs::search::{SearchQuery, SearchResults};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::file_icons::{file_icon, icon_for_file};
use crate::palette_view::SearchFlags;
use crate::theme::{Theme, mono_font, size};
use crate::title_row::icon;

/// 結果の 1 行の高さ。
const ROW_H: Pixels = px(24.);

/// 結果を 1 列に並べたときの行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    /// files の番号。
    File(usize),
    /// files の番号と、その中の lines の番号。
    Line(usize, usize),
}

/// 結果を 1 列の行に並べる（ファイルの見出しと一致の行）。折りたたんだファイルの行は出さない。
pub(crate) fn flatten(results: &SearchResults, collapsed: &HashSet<String>) -> Vec<Row> {
    let mut rows = Vec::new();
    for (i, file) in results.files.iter().enumerate() {
        rows.push(Row::File(i));
        if !collapsed.contains(file.rel.as_str()) {
            rows.extend((0..file.lines.len()).map(|j| Row::Line(i, j)));
        }
    }
    rows
}

/// 世代 got の結果を受け取るか（後から届いた古い検索の結果を捨てる）。
pub(crate) fn accept_results(current: u64, got: u64) -> bool {
    current == got
}

/// 検索の条件と切り替えから SearchQuery を作る。
pub(crate) fn make_query(pattern: &str, flags: SearchFlags) -> SearchQuery {
    SearchQuery { pattern: pattern.to_string(), case_sensitive: flags.case_sensitive, whole_word: flags.whole_word, regex: flags.regex }
}

pub(crate) fn flags_of(q: &SearchQuery) -> SearchFlags {
    SearchFlags { case_sensitive: q.case_sensitive, whole_word: q.whole_word, regex: q.regex }
}

/// タブに出す名前（20 文字で切る）。
pub(crate) fn tab_title(pattern: &str) -> String {
    const MAX: usize = 20;
    let mut title: String = pattern.chars().take(MAX).collect();
    if pattern.chars().count() > MAX {
        title.push('…');
    }
    format!("検索: {title}")
}

/// 持ち主へ知らせること。
pub(crate) enum SearchViewEvent {
    /// rel の line 行 col 桁（1 始まり）を開く。
    Open { rel: String, line: u32, col: u32 },
}

pub(crate) struct SearchView {
    /// 最後に走らせた検索。
    pub query: Option<SearchQuery>,
    results: SearchResults,
    generation: u64,
    running: bool,
    error: Option<String>,
    collapsed: HashSet<String>,
    rows: Vec<Row>,
    fs: Option<Arc<dyn ProjectFs>>,
    list: UniformListScrollHandle,
    focus: FocusHandle,
}

impl EventEmitter<SearchViewEvent> for SearchView {}

impl SearchView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        SearchView {
            query: None,
            results: SearchResults::default(),
            generation: 0,
            running: false,
            error: None,
            collapsed: HashSet::new(),
            rows: Vec::new(),
            fs: None,
            list: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// 検索を始める。前の検索の結果が後から届いても捨てる。
    pub fn run(&mut self, fs: Arc<dyn ProjectFs>, query: SearchQuery, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.query = Some(query.clone());
        self.running = true;
        self.error = None;
        self.fs = Some(fs.clone());
        let task = cx.background_spawn(async move { fs.search(&query) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if !accept_results(this.generation, generation) {
                    return;
                }
                this.running = false;
                match result {
                    Ok(results) => {
                        this.results = results;
                        this.collapsed.clear();
                        this.rows = flatten(&this.results, &this.collapsed);
                        this.list.scroll_to_item(0, ScrollStrategy::Top);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// 同じ文字列で、切り替えだけ変えて検索し直す。
    fn rerun_with(&mut self, change: impl FnOnce(&mut SearchFlags), cx: &mut Context<Self>) {
        let (Some(fs), Some(query)) = (self.fs.clone(), self.query.clone()) else { return };
        let mut flags = flags_of(&query);
        change(&mut flags);
        self.run(fs, make_query(&query.pattern, flags), cx);
    }

    fn toggle_collapsed(&mut self, file: usize, cx: &mut Context<Self>) {
        let Some(rel) = self.results.files.get(file).map(|f| f.rel.as_str().to_string()) else { return };
        if !self.collapsed.remove(&rel) {
            self.collapsed.insert(rel);
        }
        self.rows = flatten(&self.results, &self.collapsed);
        cx.notify();
    }

    fn summary(&self) -> String {
        if self.running {
            return "検索中…".into();
        }
        let matches: usize = self.results.files.iter().flat_map(|f| &f.lines).map(|l| l.ranges.len()).sum();
        let files = self.results.files.len();
        let mut text = format!("{matches} 件・{files} ファイル");
        if self.results.truncated {
            text.push_str("（一部）");
        }
        text
    }

    fn render_toggle(&self, id: &'static str, label: &'static str, on: bool, theme: &Theme, cx: &mut Context<Self>, change: fn(&mut SearchFlags)) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(22.))
            .px(px(6.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .font(mono_font())
            .text_size(px(12.))
            .text_color(if on { theme.fg_default } else { theme.fg_dim })
            .when(on, |d| d.bg(theme.bg_hover).border_1().border_color(theme.accent))
            .hover(|s| s.bg(theme.bg_hover))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.rerun_with(change, cx)))
    }

    fn render_row(&self, row: Row, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match row {
            Row::File(i) => {
                let file = &self.results.files[i];
                let rel = file.rel.as_str().to_string();
                let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
                let open = !self.collapsed.contains(&rel);
                let count: usize = file.lines.iter().map(|l| l.ranges.len()).sum();
                div()
                    .id(ElementId::Name(format!("search-file-{i}").into()))
                    .h(ROW_H)
                    .px(px(8.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .hover(|s| s.bg(theme.bg_hover))
                    .child(icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, theme.fg_dim))
                    .child(file_icon(icon_for_file(&name)))
                    .child(div().whitespace_nowrap().overflow_hidden().text_ellipsis().child(rel))
                    .child(div().flex_shrink_0().text_size(px(11.)).text_color(theme.fg_dim).child(count.to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_collapsed(i, cx)))
                    .into_any_element()
            }
            Row::Line(i, j) => {
                let file = &self.results.files[i];
                let m = &file.lines[j];
                let mark = HighlightStyle { background_color: Some(Rgba { a: 0.25, ..theme.warn }.into()), ..Default::default() };
                let highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> =
                    m.ranges.iter().map(|&(s, e)| (s as usize..e as usize, mark)).collect();
                let rel = file.rel.as_str().to_string();
                let (line, col) = (m.line, m.col);
                div()
                    .id(ElementId::Name(format!("search-line-{i}-{j}").into()))
                    .h(ROW_H)
                    .pl(px(30.))
                    .pr(px(8.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .hover(|s| s.bg(theme.bg_hover))
                    .font(mono_font())
                    .text_size(px(12.))
                    .child(div().w(px(40.)).flex_shrink_0().flex().justify_end().text_color(theme.fg_dim).child(line.to_string()))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(StyledText::new(m.text.clone()).with_highlights(highlights)),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(SearchViewEvent::Open { rel: rel.clone(), line, col })))
                    .into_any_element()
            }
        }
    }
}

impl Render for SearchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let flags = self.query.as_ref().map(flags_of).unwrap_or_default();
        let pattern = self.query.as_ref().map(|q| q.pattern.clone()).unwrap_or_default();
        let header = div()
            .flex_shrink_0()
            .px(px(10.))
            .py(px(6.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(icon(IconName::Search, theme.fg_dim))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().font(mono_font()).child(pattern))
            .child(div().flex_shrink_0().text_size(px(12.)).text_color(theme.fg_dim).child(self.summary()))
            .child(div().flex_1())
            .child(self.render_toggle("search-case", "Aa", flags.case_sensitive, &theme, cx, |f| f.case_sensitive = !f.case_sensitive))
            .child(self.render_toggle("search-word", "ab", flags.whole_word, &theme, cx, |f| f.whole_word = !f.whole_word))
            .child(self.render_toggle("search-regex", ".*", flags.regex, &theme, cx, |f| f.regex = !f.regex));
        let error = self.error.clone().map(|e| div().px(px(12.)).py(px(8.)).text_size(px(12.)).text_color(theme.danger).child(e));
        let list = uniform_list(
            "search-rows",
            self.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                let theme = *cx.global::<Theme>();
                let end = range.end.min(this.rows.len());
                this.rows[range.start.min(end)..end].to_vec().into_iter().map(|row| this.render_row(row, &theme, cx)).collect()
            }),
        )
        .track_scroll(&self.list)
        .flex_1()
        .min_h_0();
        div()
            .id("search-view")
            .track_focus(&self.focus)
            .key_context("SearchView")
            .size_full()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .text_color(theme.fg_default)
            .rounded(size::RADIUS_SM)
            .child(header)
            .children(error)
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::{Row, accept_results, flatten, tab_title};
    use fude_fs::RelPath;
    use fude_fs::search::{FileMatches, LineMatch, SearchResults};
    use std::collections::HashSet;

    fn results(files: &[(&str, usize)]) -> SearchResults {
        SearchResults {
            files: files
                .iter()
                .map(|(rel, n)| FileMatches {
                    rel: RelPath::new(rel).unwrap(),
                    lines: (0..*n).map(|i| LineMatch { line: i as u32 + 1, col: 1, text: "x".into(), ranges: vec![(0, 1)] }).collect(),
                })
                .collect(),
            truncated: false,
        }
    }

    #[test]
    fn flatten_skips_collapsed_files() {
        let r = results(&[("a.rs", 2), ("b.rs", 1)]);
        assert_eq!(flatten(&r, &HashSet::new()), [Row::File(0), Row::Line(0, 0), Row::Line(0, 1), Row::File(1), Row::Line(1, 0)]);
        let collapsed = HashSet::from(["a.rs".to_string()]);
        assert_eq!(flatten(&r, &collapsed), [Row::File(0), Row::File(1), Row::Line(1, 0)]);
    }

    #[test]
    fn stale_results_are_ignored() {
        assert!(accept_results(3, 3));
        assert!(!accept_results(3, 2));
    }

    #[test]
    fn tab_title_is_shortened() {
        assert_eq!(tab_title("fn main"), "検索: fn main");
        assert_eq!(tab_title(&"a".repeat(25)), format!("検索: {}…", "a".repeat(20)));
    }
}
