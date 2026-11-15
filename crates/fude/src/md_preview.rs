//! Markdown の整形済みのプレビュー（設計書 §5.4）。GPUI Kit の TextView を使い、
//! 相対パスの画像はプロジェクトのルートから読み、リンクはブラウザかエディタ列で開く。

use std::path::PathBuf;

use fude_core::markdown::{ImageRef, PreviewLink, resolve_image, resolve_link};
use fude_core::workspace::ProjectRoot;
use gpui_kit::base::{TextView, TextViewState, TextViewStyle};
use gpui_kit::{App, AppContext as _, Entity, Hsla, ImageSource, SharedUri, Window};

pub(crate) struct MdPreview {
    pub state: Entity<TextViewState>,
    root: ProjectRoot,
    md_rel: String,
}

impl MdPreview {
    pub(crate) fn new(text: &str, root: ProjectRoot, md_rel: String, cx: &mut App) -> MdPreview {
        let state = cx.new(|cx| TextViewState::markdown(text, cx));
        MdPreview { state, root, md_rel }
    }

    /// 中身を替える。解析は裏で行われる。
    pub(crate) fn set_text(&self, text: &str, cx: &mut App) {
        self.state.update(cx, |s, cx| s.set_text(text, cx));
    }

    pub(crate) fn set_md_rel(&mut self, rel: String) {
        self.md_rel = rel;
    }

    /// 描く要素。link はリンクの色。on_open はプロジェクトの中のリンク（ルートからの相対パス）を押したときに呼ぶ。
    pub(crate) fn element(
        &self,
        link: Hsla,
        on_open: impl Fn(String, &mut Window, &mut App) + Send + Sync + 'static,
        cx: &App,
    ) -> TextView {
        let (root, rel) = (self.root.clone(), self.md_rel.clone());
        let link_rel = self.md_rel.clone();
        TextView::new(&self.state)
            .scrollable(true)
            .selectable(true)
            .style(TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx)).with_link(link))
            .image_source(move |uri| image_source(&root, &rel, uri))
            .on_link_click(move |url, _, window, cx| match resolve_link(&link_rel, url) {
                Some(PreviewLink::External(u)) => cx.open_url(&u),
                Some(PreviewLink::Project(r)) => on_open(r, window, cx),
                None => {}
            })
    }
}

/// 画像の場所を、GPUI の画像の読み込み先にする。
fn image_source(root: &ProjectRoot, md_rel: &str, uri: &SharedUri) -> ImageSource {
    match resolve_image(md_rel, uri.as_ref()) {
        Some(ImageRef::ProjectRel(rel)) => {
            let mut path = root.host_path();
            path.extend(rel.split('/'));
            ImageSource::from(path)
        }
        Some(ImageRef::Absolute(abs)) => ImageSource::from(PathBuf::from(abs)),
        Some(ImageRef::Remote(_)) | None => ImageSource::from(uri.clone()),
    }
}
