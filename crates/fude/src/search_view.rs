//! 全体検索の結果のタブ（設計書 §5.6）。ファイルごとにまとめて出し、行を押すとそのファイルのその行を開く。
//! 検索は ProjectFs::search で裏で行う（WSL のプロジェクトは fude serve の中で走る）。

use std::collections::HashSet;
use std::sync::Arc;

use fude_fs::ProjectFs;
use fude_fs::search::{LineMatch, SearchQuery, SearchResults};
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::component::input::Input;
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

/// 置換する rel と、未保存のため飛ばす rel に分ける。
pub(crate) fn plan_replace(targets: &[String], dirty: &[String]) -> (Vec<String>, Vec<String>) {
    targets.iter().cloned().partition(|rel| !dirty.contains(rel))
}

/// 置換の文字列。置換の欄に触れた（フォーカスした、打った）あとは、空でも置換の形にする（一致を消せる）。
pub(crate) fn replacement_of(active: bool, text: &str) -> Option<String> {
    (active || !text.is_empty()).then(|| text.to_string())
}

/// 置換のプレビューの 1 片。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Span {
    Same(String),
    /// 置き換えられる元の部分（取り消し線）。
    Old(String),
    /// 置き換えたあとの部分。
    New(String),
}

/// プレビュー: 1 行の表示用の本文を、元の部分と新しい部分の並びにする。正規表現のときは一致ごとに $1 などを展開する。
pub(crate) fn preview_spans(m: &LineMatch, q: &SearchQuery, replacement: &str) -> Vec<Span> {
    let re = if q.regex { fude_fs::search::compile_query(q).ok() } else { None };
    let text = m.text.as_str();
    let mut out = Vec::new();
    let mut at = 0;
    for &(s, e) in &m.ranges {
        let (s, e) = (s as usize, e as usize);
        if s < at || e > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
            continue;
        }
        if s > at {
            out.push(Span::Same(text[at..s].to_string()));
        }
        out.push(Span::Old(text[s..e].to_string()));
        let new = re
            .as_ref()
            .and_then(|re| re.captures_at(text, s))
            .filter(|c| c.get(0).is_some_and(|g| g.start() == s))
            .map(|c| {
                let mut dst = String::new();
                c.expand(replacement, &mut dst);
                dst
            })
            .unwrap_or_else(|| replacement.to_string());
        out.push(Span::New(new));
        at = e;
    }
    if at < text.len() {
        out.push(Span::Same(text[at..].to_string()));
    }
    out
}

/// 持ち主へ知らせること。
pub(crate) enum SearchViewEvent {
    /// rel の line 行 col 桁（1 始まり）を開く。
    Open { rel: String, line: u32, col: u32 },
    /// rels の一致を replacement で置換する。confirm なら先に確かめる。
    ReplaceFiles { rels: Vec<String>, replacement: String, confirm: bool },
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
    /// 置換の文字列の入力欄。
    replace: Entity<InputState>,
    /// 置換の欄に触れたか。
    replace_active: bool,
    _replace_sub: Subscription,
}

impl EventEmitter<SearchViewEvent> for SearchView {}

impl SearchView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let replace = cx.new(|cx| InputState::new(window, cx).placeholder("置換"));
        let _replace_sub = cx.subscribe(&replace, |this: &mut SearchView, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change | InputEvent::Focus) {
                this.replace_active = true;
                cx.notify();
            }
        });
        SearchView {
            replace,
            replace_active: false,
            _replace_sub,
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

    /// rels の一致の数。
    pub fn match_count(&self, rels: &[String]) -> usize {
        self.results
            .files
            .iter()
            .filter(|f| rels.iter().any(|r| r == f.rel.as_str()))
            .flat_map(|f| &f.lines)
            .map(|l| l.ranges.len())
            .sum()
    }

    /// 置換の欄の文字列（空なら置換しない）。
    fn replacement(&self, cx: &App) -> Option<String> {
        replacement_of(self.replace_active, &self.replace.read(cx).value())
    }

    fn emit_replace(&mut self, rels: Vec<String>, confirm: bool, cx: &mut Context<Self>) {
        let Some(replacement) = self.replacement(cx) else { return };
        if rels.is_empty() || self.running {
            return;
        }
        cx.emit(SearchViewEvent::ReplaceFiles { rels, replacement, confirm });
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
        let replacement = self.replacement(cx);
        match row {
            Row::File(i) => {
                let file = &self.results.files[i];
                let rel = file.rel.as_str().to_string();
                let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
                let open = !self.collapsed.contains(&rel);
                let rel_for_button = rel.clone();
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
                    .child(div().flex_1())
                    .when(replacement.is_some(), |d| {
                        let rel = rel_for_button.clone();
                        d.child(
                            text_button(ElementId::Name(format!("search-replace-{i}").into()), "置換", theme)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.emit_replace(vec![rel.clone()], false, cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_collapsed(i, cx)))
                    .into_any_element()
            }
            Row::Line(i, j) => {
                let file = &self.results.files[i];
                let m = &file.lines[j];
                let mark = HighlightStyle { background_color: Some(Rgba { a: 0.25, ..theme.warn }.into()), ..Default::default() };
                let text: StyledText = match (&replacement, &self.query) {
                    (Some(r), Some(q)) => preview_text(&preview_spans(m, q, r), theme),
                    _ => {
                        let highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> =
                            m.ranges.iter().map(|&(s, e)| (s as usize..e as usize, mark)).collect();
                        StyledText::new(m.text.clone()).with_highlights(highlights)
                    }
                };
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
                            .child(text),
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
        let can_replace = self.replacement(cx).is_some() && !self.results.files.is_empty() && !self.running;
        let replace_row = div()
            .flex_shrink_0()
            .px(px(10.))
            .py(px(4.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(div().flex_1().min_w_0().font(mono_font()).child(Input::new(&self.replace).appearance(false).prefix(icon(IconName::Replace, theme.fg_dim))))
            .child(
                text_button("search-replace-all", "すべて置換", &theme)
                    .when(!can_replace, |d| d.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let rels = this.results.files.iter().map(|f| f.rel.as_str().to_string()).collect();
                        this.emit_replace(rels, true, cx);
                    })),
            );
        let error = self.error.clone().map(|e| div().px(px(12.)).py(px(8.)).text_size(px(12.)).text_color(theme.danger).child(e));
        let list = uniform_list(
            "search-rows",
            self.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                let theme = *cx.global::<Theme>();
                let end = range.end.min(this.rows.len());
                this.rows[range.start.min(end)..end].iter().copied().map(|row| this.render_row(row, &theme, cx)).collect()
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
            .child(replace_row)
            .children(error)
            .child(list)
    }
}

/// 小さな文字のボタン（「置換」「すべて置換」）。
fn text_button(id: impl Into<ElementId>, label: &'static str, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .h(px(20.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(theme.border_subtle)
        .text_size(px(11.))
        .text_color(theme.fg_default)
        .hover(|s| s.bg(theme.bg_hover))
        .child(label)
}

/// プレビューの片を 1 つの文字列にし、元の部分に取り消し線、新しい部分に diff_add の色を付ける。
fn preview_text(spans: &[Span], theme: &Theme) -> StyledText {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for span in spans {
        let start = text.len();
        let style = match span {
            Span::Same(s) => {
                text.push_str(s);
                None
            }
            Span::Old(s) => {
                text.push_str(s);
                Some(HighlightStyle {
                    color: Some(theme.fg_dim.into()),
                    background_color: Some(Rgba { a: 0.2, ..theme.diff_del }.into()),
                    strikethrough: Some(StrikethroughStyle { color: Some(theme.fg_dim.into()), thickness: px(1.) }),
                    ..Default::default()
                })
            }
            Span::New(s) => {
                text.push_str(s);
                Some(HighlightStyle {
                    color: Some(theme.diff_add.into()),
                    background_color: Some(Rgba { a: 0.2, ..theme.diff_add }.into()),
                    ..Default::default()
                })
            }
        };
        if let Some(style) = style
            && text.len() > start
        {
            highlights.push((start..text.len(), style));
        }
    }
    StyledText::new(text).with_highlights(highlights)
}

#[cfg(test)]
mod tests {
    use super::{Row, Span, accept_results, flatten, plan_replace, preview_spans, replacement_of, tab_title};
    use fude_fs::RelPath;
    use fude_fs::search::{FileMatches, LineMatch, SearchQuery, SearchResults};
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
    fn empty_replacement_is_allowed_once_replace_is_used() {
        // 一致を消す（空の文字列で置換する）ことができる。置換の欄に触れるまでは置換の形にしない。
        assert_eq!(replacement_of(false, ""), None);
        assert_eq!(replacement_of(true, ""), Some(String::new()));
        assert_eq!(replacement_of(false, "x"), Some("x".into()));
    }

    #[test]
    fn replace_skips_dirty_files() {
        let (go, skip) = plan_replace(&["a.rs".into(), "b.rs".into()], &["b.rs".into()]);
        assert_eq!(go, ["a.rs"]);
        assert_eq!(skip, ["b.rs"]);
    }

    #[test]
    fn preview_marks_old_and_new() {
        let m = LineMatch { line: 1, col: 5, text: "let foo = foo;".into(), ranges: vec![(4, 7), (10, 13)] };
        let q = SearchQuery { pattern: "foo".into(), ..Default::default() };
        assert_eq!(
            preview_spans(&m, &q, "bar"),
            [
                Span::Same("let ".into()),
                Span::Old("foo".into()),
                Span::New("bar".into()),
                Span::Same(" = ".into()),
                Span::Old("foo".into()),
                Span::New("bar".into()),
                Span::Same(";".into()),
            ]
        );
        // 正規表現のときは、一致ごとに $1 を展開した文字列を出す。
        let re = SearchQuery { pattern: r"(\w)oo".into(), regex: true, ..Default::default() };
        let m = LineMatch { line: 1, col: 1, text: "foo".into(), ranges: vec![(0, 3)] };
        assert_eq!(preview_spans(&m, &re, "${1}X"), [Span::Old("foo".into()), Span::New("fX".into())]);
    }

    #[test]
    fn tab_title_is_shortened() {
        assert_eq!(tab_title("fn main"), "検索: fn main");
        assert_eq!(tab_title(&"a".repeat(25)), format!("検索: {}…", "a".repeat(20)));
    }
}
