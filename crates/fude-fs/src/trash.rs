//! OS のゴミ箱に送る、ゴミ箱から戻す。Windows と Linux は trash クレート、macOS は NSFileManager。
//!
//! 札（TrashTicket）の id は、Windows と Linux ではゴミ箱の項目の id、macOS ではゴミ箱の中の絶対パス。
//! ゴミ箱には入ったが札が取れなかったときは、id が空の札を返す（取り消しには使えない）。

use std::fs;
use std::io;
use std::path::Path;

use crate::TrashTicket;

#[cfg(any(windows, target_os = "linux"))]
mod imp {
    use std::collections::HashSet;
    use std::fs;
    use std::io;
    use std::path::Path;

    use trash::TrashItem;
    use trash::os_limited::{list, restore_all};

    use crate::TrashTicket;

    fn trash_error(e: trash::Error) -> io::Error {
        match e {
            trash::Error::RestoreCollision { path, .. } => {
                io::Error::new(io::ErrorKind::AlreadyExists, format!("すでにある: {}", path.display()))
            }
            e => io::Error::other(e.to_string()),
        }
    }

    fn item_id(item: &TrashItem) -> String {
        item.id.to_string_lossy().into_owned()
    }

    /// 比べるための形。Windows は \\?\ を外し、区切りを揃え、大文字小文字を区別しない。
    fn path_key(p: &Path) -> String {
        let s = p.to_string_lossy();
        if cfg!(windows) {
            s.strip_prefix(r"\\?\").unwrap_or(&s).replace('/', "\\").trim_end_matches('\\').to_lowercase()
        } else {
            s.trim_end_matches('/').to_string()
        }
    }

    /// 元の場所として考えられる形（そのまま、親を正規化したもの）。
    fn path_keys(path: &Path) -> Vec<String> {
        let mut keys = vec![path_key(path)];
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
            && let Ok(parent) = fs::canonicalize(parent)
        {
            keys.push(path_key(&parent.join(name)));
        }
        keys
    }

    fn original_key(item: &TrashItem) -> String {
        path_key(&item.original_parent.join(&item.name))
    }

    pub fn move_to_trash(path: &Path) -> io::Result<TrashTicket> {
        fs::symlink_metadata(path)?;
        let keys = path_keys(path);
        let before: Option<HashSet<String>> = list().ok().map(|items| items.iter().map(item_id).collect());
        trash::delete(path).map_err(trash_error)?;
        let id = before
            .and_then(|before| {
                let after = list().ok()?;
                after
                    .into_iter()
                    .filter(|item| !before.contains(&item_id(item)) && keys.contains(&original_key(item)))
                    .max_by_key(|item| item.time_deleted)
                    .map(|item| item_id(&item))
            })
            .unwrap_or_default();
        Ok(TrashTicket { id })
    }

    pub fn restore_from_trash(ticket: &TrashTicket, original: &Path) -> io::Result<()> {
        let not_found = || io::Error::new(io::ErrorKind::NotFound, "ゴミ箱に見つからない");
        if ticket.id.is_empty() {
            return Err(not_found());
        }
        let item = list().map_err(trash_error)?.into_iter().find(|item| item_id(item) == ticket.id).ok_or_else(not_found)?;
        if !path_keys(original).contains(&original_key(&item)) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "元の場所と違う場所には戻せない"));
        }
        if fs::symlink_metadata(original).is_ok() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("すでにある: {}", original.display())));
        }
        restore_all([item]).map_err(trash_error)
    }
}

#[cfg(target_os = "macos")]
#[allow(unused_unsafe)]
mod imp {
    use std::fs;
    use std::io;
    use std::path::Path;

    use objc2::rc::Retained;
    use objc2_foundation::{NSFileManager, NSString, NSURL};

    use crate::TrashTicket;

    pub fn move_to_trash(path: &Path) -> io::Result<TrashTicket> {
        fs::symlink_metadata(path)?;
        let id = unsafe {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            let manager = NSFileManager::defaultManager();
            let mut resulting: Option<Retained<NSURL>> = None;
            manager
                .trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting))
                .map_err(|e| io::Error::other(e.localizedDescription().to_string()))?;
            resulting.and_then(|u| u.path()).map(|p| p.to_string()).unwrap_or_default()
        };
        Ok(TrashTicket { id })
    }

    pub fn restore_from_trash(ticket: &TrashTicket, original: &Path) -> io::Result<()> {
        let inside = Path::new(&ticket.id);
        if ticket.id.is_empty() || fs::symlink_metadata(inside).is_err() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "ゴミ箱に見つからない"));
        }
        if fs::symlink_metadata(original).is_ok() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("すでにある: {}", original.display())));
        }
        fs::rename(inside, original)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    use std::io;
    use std::path::Path;

    use crate::TrashTicket;

    pub fn move_to_trash(_path: &Path) -> io::Result<TrashTicket> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "この OS ではゴミ箱を使えない"))
    }

    pub fn restore_from_trash(_ticket: &TrashTicket, _original: &Path) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "この OS ではゴミ箱を使えない"))
    }
}

/// path をゴミ箱に送る。無ければ NotFound。
pub(crate) fn move_to_trash(path: &Path) -> io::Result<TrashTicket> {
    imp::move_to_trash(path)
}

/// 札の項目を original（元の場所）へ戻す。埋まっていれば AlreadyExists、ゴミ箱に無ければ NotFound。
pub(crate) fn restore_from_trash(ticket: &TrashTicket, original: &Path) -> io::Result<()> {
    if let Some(parent) = original.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    imp::restore_from_trash(ticket, original)
}
