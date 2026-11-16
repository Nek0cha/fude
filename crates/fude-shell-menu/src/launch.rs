//! 「fude で開く」を押したときに起動する fude.exe の場所と、その起動のしかた（OS に依らない部分）。

use std::path::{Path, PathBuf};
use std::process::Command;

/// DLL の横の `fude-path.txt` の中身から fude.exe のパスを取り出す。BOM と前後の空白・改行は無視し、空なら None。
pub fn parse_fude_path(text: &str) -> Option<PathBuf> {
    let path = text.trim_start_matches('\u{feff}').trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// `fude.exe <対象のパス>` を、窓を出さずに起動するコマンドにする。対象は 1 つの引数として渡す。
/// 引用符の付け方（末尾の `\` を含む）は std に任せる。
pub fn launch_command(exe: &Path, target: &Path) -> Command {
    let mut command = Command::new(exe);
    command.arg(target);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 開発用の console の fude.exe でも窓を出さない。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fude_path_strips_bom_and_newline() {
        assert_eq!(parse_fude_path("\u{feff}C:\\Users\\a b\\fude.exe\r\n"), Some(PathBuf::from(r"C:\Users\a b\fude.exe")));
        assert_eq!(parse_fude_path("  \r\n"), None);
        assert_eq!(parse_fude_path(""), None);
    }

    #[test]
    fn launch_command_passes_target_as_one_arg() {
        for t in [r"C:\", r"\\wsl.localhost\Ubuntu\", r"C:\作業 フォルダ\a.md"] {
            let c = launch_command(Path::new(r"C:\fude\fude.exe"), Path::new(t));
            assert_eq!(c.get_program(), r"C:\fude\fude.exe");
            assert_eq!(c.get_args().collect::<Vec<_>>(), vec![std::ffi::OsStr::new(t)]);
        }
    }
}
