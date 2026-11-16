//! fude が使う Lucide のアイコンだけを埋め込む。ファイルアイコン（Material Icon Theme）は build.rs が埋め込む。

use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString};

gpui_kit::assets::icon_assets!(pub FudeAssets, [
    Plus,
    X,
    Terminal,
    Columns2,
    GitBranch,
    WindowMinimize,
    WindowMaximize,
    WindowRestore,
    WindowClose,
    ArrowDown,
    FolderOpen,
    Copy,
    RotateCcwClock,
    ChevronLeft,
    ChevronRight,
    ArrowUp,
    FileDiff,
    Search,
    ChevronDown,
    File,
    FilePlus,
    FolderPlus,
    Pencil,
    Trash,
    Replace,
    Bell,
    Check,
    ListTodo,
    Send,
    Pin,
    Sparkles,
]);

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/file_icons.rs"));
}

/// アプリのアセット。`file-icons/` はファイルアイコン、それ以外は FudeAssets に任せる。
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(name) = path.strip_prefix("file-icons/") {
            return Ok(embedded::FILE_ICONS.iter().find(|(n, _)| *n == name).map(|(_, bytes)| Cow::Borrowed(*bytes)));
        }
        FudeAssets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        if path.starts_with("file-icons") {
            return Ok(embedded::FILE_ICONS.iter().map(|(n, _)| format!("file-icons/{n}").into()).collect());
        }
        FudeAssets.list(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_mapped_icons_are_embedded() {
        for name in crate::file_icons::ICONS {
            let path = crate::file_icons::icon_path(name);
            assert!(AppAssets.load(&path).unwrap().is_some(), "{path}");
        }
        assert!(AppAssets.load("file-icons/none.svg").unwrap().is_none());
        // Lucide のアイコンは今までどおり読める。
        assert!(AppAssets.load("icons/plus.svg").unwrap().is_some());
    }

    #[test]
    fn claude_icon_is_generic() {
        // Claude のロゴは Anthropic の書面の許可が要るので使わない。Lucide の sparkles を使う。
        assert!(AppAssets.load("icons/sparkles.svg").unwrap().is_some());
        assert!(AppAssets.load("brand/claude.svg").unwrap().is_none());
    }
}
