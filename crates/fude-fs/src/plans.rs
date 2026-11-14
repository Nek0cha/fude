//! プランのファイル（docs/superpowers/plans/*.md）と、その台帳を読む（設計書 §5.8）。
//! このフォルダは gitignore されていることが多く、ツリーや監視では見えないため、ignore を通さずに直接読む。

use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::RelPath;

pub const PLANS_DIR: &str = "docs/superpowers/plans";
pub const LEDGER_DIR: &str = ".superpowers/sdd";

/// 読んだプランと台帳の印。同じなら読み直さない。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStamp {
    pub rel: RelPath,
    pub plan_mtime_ms: u64,
    pub ledger_mtime_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanFetch {
    /// プランが無い。
    None,
    /// known と同じ。
    Unchanged,
    Changed { stamp: PlanStamp, plan: String, ledger: Option<String> },
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64)
}

/// root の下で、PLANS_DIR の .md のうち最も新しいもの（同じなら名前の大きいもの）と、その台帳
/// （LEDGER_DIR/<拡張子なしの名前>/progress.md）。フォルダが無ければ None。印が known と同じなら Unchanged。
/// 中身は UTF-8 でなければ置き換えて読む。
pub fn plan_files(root: &Path, known: Option<&PlanStamp>) -> io::Result<PlanFetch> {
    let dir = root.join(PLANS_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(PlanFetch::None),
        Err(e) => return Err(e),
    };
    let mut newest: Option<(u64, String)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let key = (mtime_ms(&meta), name);
        if newest.as_ref().is_none_or(|n| key > *n) {
            newest = Some(key);
        }
    }
    let Some((plan_mtime_ms, name)) = newest else { return Ok(PlanFetch::None) };
    let stem = name.strip_suffix(".md").unwrap_or(&name);
    let ledger_path = root.join(LEDGER_DIR).join(stem).join("progress.md");
    let ledger_mtime_ms = std::fs::metadata(&ledger_path).ok().filter(|m| m.is_file()).map(|m| mtime_ms(&m));
    let stamp = PlanStamp { rel: RelPath::new(&format!("{PLANS_DIR}/{name}"))?, plan_mtime_ms, ledger_mtime_ms };
    if known == Some(&stamp) {
        return Ok(PlanFetch::Unchanged);
    }
    let plan = String::from_utf8_lossy(&std::fs::read(dir.join(&name))?).into_owned();
    let ledger = match ledger_mtime_ms {
        Some(_) => std::fs::read(&ledger_path).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
        None => None,
    };
    Ok(PlanFetch::Changed { stamp, plan, ledger })
}

#[cfg(test)]
mod tests {
    use super::{PlanFetch, plan_files};
    use std::path::Path;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn picks_newest_plan_and_ledger() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "docs/superpowers/plans/a.md", "# A\n");
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(dir.path(), "docs/superpowers/plans/b.md", "# B\n");
        write(dir.path(), "docs/superpowers/plans/notes.txt", "x");
        write(dir.path(), ".superpowers/sdd/b/progress.md", "Task 1: complete\n");
        let PlanFetch::Changed { stamp, plan, ledger } = plan_files(dir.path(), None).unwrap() else { panic!() };
        assert_eq!(stamp.rel.as_str(), "docs/superpowers/plans/b.md");
        assert_eq!(plan, "# B\n");
        assert_eq!(ledger.as_deref(), Some("Task 1: complete\n"));
        assert_eq!(plan_files(dir.path(), Some(&stamp)).unwrap(), PlanFetch::Unchanged);
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(dir.path(), ".superpowers/sdd/b/progress.md", "Task 1: complete\nTask 2: complete\n");
        assert!(matches!(plan_files(dir.path(), Some(&stamp)).unwrap(), PlanFetch::Changed { .. }));
    }

    #[test]
    fn no_plans_dir_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(plan_files(dir.path(), None).unwrap(), PlanFetch::None);
        write(dir.path(), "docs/superpowers/plans/x.txt", "x");
        assert_eq!(plan_files(dir.path(), None).unwrap(), PlanFetch::None);
    }

    #[test]
    fn ignored_plans_are_still_found() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".gitignore", "/docs/superpowers/\n/.superpowers/\n");
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        write(dir.path(), "docs/superpowers/plans/p.md", "# P\n");
        assert!(matches!(plan_files(dir.path(), None).unwrap(), PlanFetch::Changed { ledger: None, .. }));
    }
}
