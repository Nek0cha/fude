//! ツリーの項目のドラッグ（設計書 §5.2、§5.4）。Markdown には画像のリンクを、ターミナルにはパスを入れる。

use std::path::PathBuf;

use fude_core::location::{FileLocation, ProjectRoot};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::file_icons::{file_icon, icon_for_file};
use crate::theme::{Theme, popup_shadow, size, ui_font};

/// ツリーから運ぶもの。
#[derive(Clone)]
pub(crate) struct DraggedEntry {
    pub rel: String,
    pub is_dir: bool,
    pub location: FileLocation,
}

impl Render for DraggedEntry {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let name = self.rel.rsplit('/').next().unwrap_or(&self.rel).to_string();
        div()
            .px(px(8.))
            .h(px(24.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .rounded(size::RADIUS_SM)
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border_subtle)
            .shadow(popup_shadow())
            .font(ui_font())
            .text_size(px(12.))
            .text_color(theme.fg_default)
            .when(!self.is_dir, |d| d.child(file_icon(icon_for_file(&name))))
            .child(name)
    }
}

/// OS から落とされたパスを場所にする（`\\?\` を外し、WSL の UNC は WSL の場所にする）。読めないものは捨てる。
pub(crate) fn external_locations(paths: &ExternalPaths) -> Vec<FileLocation> {
    paths
        .paths()
        .iter()
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .map(|p| FileLocation::from_host_path(fude_core::paths::simplify_verbatim(p)))
        .collect()
}

/// ルート root の中の rel（"/" 区切り）の場所。
pub(crate) fn entry_location(root: &ProjectRoot, rel: &str) -> FileLocation {
    match root {
        ProjectRoot::Local(dir) => {
            let mut path = PathBuf::from(dir);
            path.extend(rel.split('/').filter(|s| !s.is_empty()));
            FileLocation::Local(path)
        }
        ProjectRoot::Wsl { distro, path } => {
            let base = path.trim_end_matches('/');
            FileLocation::Wsl { distro: distro.clone(), path: format!("{base}/{rel}") }
        }
    }
}

/// エディタの本文の上端から y（ピクセル）の所に落としたときの行（0 始まり）。scroll_y は上へ送った分（負）。
/// 行の位置が分からないときの目安で、折り返しは数えない。最後の行より下は line_count（末尾）。
pub(crate) fn drop_line(y_in_text: f32, scroll_y: f32, line_height: f32, line_count: u32) -> u32 {
    if line_height <= 0.0 {
        return line_count;
    }
    let row = ((y_in_text - scroll_y) / line_height).floor();
    if row < 0.0 { 0 } else { (row as u32).min(line_count) }
}

#[cfg(test)]
mod tests {
    use super::{drop_line, entry_location};
    use fude_core::location::{FileLocation, ProjectRoot};
    use std::path::PathBuf;

    #[test]
    fn drop_line_from_point() {
        // 行の高さ 20px、スクロール無し。
        assert_eq!(drop_line(0.0, 0.0, 20.0, 10), 0);
        assert_eq!(drop_line(45.0, 0.0, 20.0, 10), 2);
        // 40px 上へ送っている（scroll_y は負）。
        assert_eq!(drop_line(5.0, -40.0, 20.0, 10), 2);
        // 最後の行より下は最後の行の次（末尾に足す）。
        assert_eq!(drop_line(900.0, 0.0, 20.0, 10), 10);
        assert_eq!(drop_line(-30.0, 0.0, 20.0, 10), 0);
    }

    #[test]
    fn entry_locations() {
        let local = ProjectRoot::Local(PathBuf::from("/w/app"));
        assert_eq!(entry_location(&local, "img/a.png"), FileLocation::Local(PathBuf::from("/w/app").join("img").join("a.png")));
        let wsl = ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/home/me".into() };
        assert_eq!(entry_location(&wsl, "a b.png"), FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/me/a b.png".into() });
        let top = ProjectRoot::Wsl { distro: "Ubuntu".into(), path: "/".into() };
        assert_eq!(entry_location(&top, "etc/x"), FileLocation::Wsl { distro: "Ubuntu".into(), path: "/etc/x".into() });
    }
}
