//! WSL の中で、ターミナル（FUDE_TERMINAL_ID）のシェルの上で何かが動いているかを /proc から調べる。

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

/// proc_dir（ふつうは /proc）の中で、FUDE_TERMINAL_ID が terminal_id のプロセスのうち、親も同じ id を持つもの
/// （シェルの子孫）が 1 つでもあれば真。シェル自身は親（ターミナルの外の wsl のプロセス）が id を持たない。
/// 読めないプロセス（終わった、権限が無い）は飛ばす。
pub fn terminal_busy(proc_dir: &Path, terminal_id: &str) -> io::Result<bool> {
    let wanted = format!("FUDE_TERMINAL_ID={terminal_id}");
    // pid → 親の pid。id を持つプロセスだけを入れる。
    let mut marked: HashMap<u32, u32> = HashMap::new();
    for entry in fs::read_dir(proc_dir)? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(environ) = fs::read(entry.path().join("environ")) else { continue };
        if !environ.split(|b| *b == 0).any(|var| var == wanted.as_bytes()) {
            continue;
        }
        let Some(ppid) = fs::read_to_string(entry.path().join("stat")).ok().and_then(|s| parent_pid(&s)) else {
            continue;
        };
        marked.insert(pid, ppid);
    }
    Ok(marked.values().any(|ppid| marked.contains_key(ppid)))
}

/// /proc/<pid>/stat の 4 番目の欄（親の pid）。2 番目の欄（名前）は括弧つきで空白を含むことがあるため、最後の ")" の後ろから数える。
fn parent_pid(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// 偽の /proc に、pid、親の pid、FUDE_TERMINAL_ID を持つプロセスを作る。
    fn proc_entry(dir: &Path, pid: u32, ppid: u32, id: Option<&str>) {
        let p = dir.join(pid.to_string());
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("stat"), format!("{pid} (sh) S {ppid} {pid} {pid} 0 -1")).unwrap();
        let mut env = b"PATH=/bin\0".to_vec();
        if let Some(id) = id {
            env.extend_from_slice(format!("FUDE_TERMINAL_ID={id}\0").as_bytes());
        }
        fs::write(p.join("environ"), env).unwrap();
    }

    #[test]
    fn shell_alone_is_not_busy() {
        let dir = tempfile::tempdir().unwrap();
        proc_entry(dir.path(), 1, 0, None);
        proc_entry(dir.path(), 10, 1, Some("p1"));
        assert!(!terminal_busy(dir.path(), "p1").unwrap());
    }

    #[test]
    fn child_of_shell_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        proc_entry(dir.path(), 1, 0, None);
        proc_entry(dir.path(), 10, 1, Some("p1"));
        proc_entry(dir.path(), 11, 10, Some("p1"));
        assert!(terminal_busy(dir.path(), "p1").unwrap());
        assert!(!terminal_busy(dir.path(), "p2").unwrap());
    }

    #[test]
    fn unreadable_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("self")).unwrap();
        fs::create_dir_all(dir.path().join("12")).unwrap();
        fs::write(dir.path().join("version"), "x").unwrap();
        proc_entry(dir.path(), 10, 1, Some("p1"));
        assert!(!terminal_busy(dir.path(), "p1").unwrap());
    }

    #[test]
    fn prefix_id_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        proc_entry(dir.path(), 10, 1, Some("p1"));
        proc_entry(dir.path(), 11, 10, Some("p10"));
        assert!(!terminal_busy(dir.path(), "p1").unwrap());
    }
}
