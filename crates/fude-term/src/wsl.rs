//! WSL のディストリビューションの一覧。

/// `wsl.exe -l -q` の出力を読む。NUL を含めば UTF-16LE として読む（WSL_UTF8 が効かない版のため）。
/// BOM と空行を除き、前後の空白を削る。
pub fn parse_distro_list(stdout: &[u8]) -> Vec<String> {
    let text = if stdout.contains(&0) {
        let units: Vec<u16> = stdout.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(stdout).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}'))
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// インストール済みのディストリビューション。Windows 以外では空。
pub fn list_distros() -> std::io::Result<Vec<String>> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// コンソールの窓を出さない。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("wsl.exe")
            .args(["-l", "-q"])
            .env("WSL_UTF8", "1")
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;
        if !out.status.success() {
            let msg = parse_distro_list(&out.stdout).join(" ");
            return Err(std::io::Error::other(format!("wsl.exe -l -q が失敗した: {msg}")));
        }
        Ok(parse_distro_list(&out.stdout))
    }
    #[cfg(not(windows))]
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_utf16_list() {
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend("Ubuntu\r\nDebian\r\n".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(parse_distro_list(&bytes), vec!["Ubuntu", "Debian"]);
    }

    #[test]
    fn parse_utf8_list() {
        assert_eq!(parse_distro_list(b"Ubuntu\n\n"), vec!["Ubuntu"]);
    }
}
