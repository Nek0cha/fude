//! ターミナルに落としたファイルのパスを、シェルに入力する文字列にする（設計書 §5.2）。

use crate::keymap::Platform;
use crate::location::FileLocation;
use crate::workspace::ShellKind;

/// 落としたファイルを、空白でつないだパスにする（末尾に空白を 1 つ）。要るものだけ引用符で囲む。
/// WSL のシェルへは Linux 形式（Windows のドライブは /mnt/<d>）、ネイティブのシェルへは WSL のパスを UNC にする。
pub fn drop_text(files: &[FileLocation], shell: &ShellKind, platform: Platform) -> String {
    let mut out = String::new();
    for file in files {
        let path = shell_path(file, shell);
        let quoted = match (shell, platform) {
            (ShellKind::Native, Platform::Windows) => quote_pwsh(&path),
            _ => quote_posix(&path),
        };
        out.push_str(&quoted);
        out.push(' ');
    }
    out
}

fn shell_path(file: &FileLocation, shell: &ShellKind) -> String {
    match (file, shell) {
        (FileLocation::Local(p), ShellKind::Wsl { .. }) => windows_to_mnt(&p.to_string_lossy()),
        (FileLocation::Local(p), ShellKind::Native) => p.to_string_lossy().into_owned(),
        (FileLocation::Wsl { distro, path }, ShellKind::Wsl { distro: d }) if distro.eq_ignore_ascii_case(d) => path.clone(),
        (FileLocation::Wsl { distro, path }, _) => format!(r"\\wsl.localhost\{distro}{}", path.replace('/', "\\")),
    }
}

/// `C:\a\b` → `/mnt/c/a/b`。ドライブで始まらなければ `\` を `/` にするだけ。
fn windows_to_mnt(path: &str) -> String {
    let b = path.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        format!("/mnt/{}{}", (b[0] as char).to_ascii_lowercase(), path[2..].replace('\\', "/"))
    } else {
        path.replace('\\', "/")
    }
}

/// PowerShell: 空白か特別な文字を含むときだけ '…' で囲み、中の ' は '' にする。
fn quote_pwsh(path: &str) -> String {
    let special = |c: char| c.is_whitespace() || "'\"$`&(){}[];,@#|<>".contains(c);
    if path.chars().any(special) { format!("'{}'", path.replace('\'', "''")) } else { path.to_string() }
}

/// bash / zsh / fish: 安全な文字だけなら囲まない。それ以外は '…' で囲み、中の ' は '\'' にする。
fn quote_posix(path: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./~+:=@%^,-".contains(c);
    if !path.is_empty() && path.chars().all(safe) { path.to_string() } else { format!("'{}'", path.replace('\'', r"'\''")) }
}

#[cfg(test)]
mod tests {
    use super::drop_text;
    use crate::keymap::Platform;
    use crate::location::FileLocation;
    use crate::workspace::ShellKind;
    use std::path::PathBuf;

    #[test]
    fn pwsh_quotes_only_when_needed() {
        let f = |p: &str| FileLocation::Local(PathBuf::from(p));
        assert_eq!(drop_text(&[f(r"C:\w\a.rs")], &ShellKind::Native, Platform::Windows), r"C:\w\a.rs ");
        assert_eq!(drop_text(&[f(r"C:\w\my file.png")], &ShellKind::Native, Platform::Windows), r"'C:\w\my file.png' ");
        assert_eq!(drop_text(&[f(r"C:\w\it's.png")], &ShellKind::Native, Platform::Windows), r"'C:\w\it''s.png' ");
        assert_eq!(drop_text(&[f(r"C:\a"), f(r"C:\b")], &ShellKind::Native, Platform::Windows), r"C:\a C:\b ");
    }

    #[test]
    fn posix_quotes_single_quote() {
        let f = |p: &str| FileLocation::Local(PathBuf::from(p));
        assert_eq!(drop_text(&[f("/Users/me/a b.png")], &ShellKind::Native, Platform::MacOs), "'/Users/me/a b.png' ");
        assert_eq!(drop_text(&[f("/Users/me/it's")], &ShellKind::Native, Platform::MacOs), r"'/Users/me/it'\''s' ");
        assert_eq!(drop_text(&[f("/Users/me/日本.md")], &ShellKind::Native, Platform::MacOs), "'/Users/me/日本.md' ");
    }

    #[test]
    fn wsl_shell_gets_linux_paths() {
        let ubuntu = ShellKind::Wsl { distro: "Ubuntu".into() };
        let local = FileLocation::Local(PathBuf::from(r"C:\Users\me\a b.png"));
        assert_eq!(drop_text(&[local], &ubuntu, Platform::Windows), "'/mnt/c/Users/me/a b.png' ");
        let same = FileLocation::Wsl { distro: "Ubuntu".into(), path: "/home/me/x.rs".into() };
        assert_eq!(drop_text(&[same.clone()], &ubuntu, Platform::Windows), "/home/me/x.rs ");
        assert_eq!(drop_text(&[same], &ShellKind::Native, Platform::Windows), r"\\wsl.localhost\Ubuntu\home\me\x.rs ");
        let other = FileLocation::Wsl { distro: "Debian".into(), path: "/x".into() };
        assert_eq!(drop_text(&[other], &ubuntu, Platform::Windows), r"'\\wsl.localhost\Debian\x' ");
    }
}
