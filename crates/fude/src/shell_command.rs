//! macOS のシェルコマンド（`/usr/local/bin/fude` のリンク）をインストール・削除するための判断（10d §5）。
//! 実行（osascript）は WorkspaceView が行う。ここは全 OS でコンパイル・試験できる関数だけを置く。

use std::path::{Path, PathBuf};

/// シェルコマンドのリンクの場所。
pub(crate) const LINK_PATH: &str = "/usr/local/bin/fude";

/// exe が `<名前>.app/Contents/MacOS/` の中にあるときだけ Some(exe)。
pub(crate) fn bundle_executable(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let is_app = app.extension().is_some_and(|e| e == "app");
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && is_app).then(|| exe.to_path_buf())
}

/// リンクの場所にあるもの。
#[derive(Debug, PartialEq)]
pub(crate) enum LinkState {
    Missing,
    Symlink(PathBuf),
    /// シンボリックリンクではないもの（普通のファイルやフォルダ）。fude のものではないので触らない。
    Other,
}

pub(crate) fn link_state(path: &Path) -> LinkState {
    match std::fs::symlink_metadata(path) {
        Err(_) => LinkState::Missing,
        Ok(m) if m.file_type().is_symlink() => match std::fs::read_link(path) {
            Ok(target) => LinkState::Symlink(target),
            Err(_) => LinkState::Other,
        },
        Ok(_) => LinkState::Other,
    }
}

/// シェルの単一引用で包む（`'` は `'\''` にする）。
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// シェルの文を、管理者の権限で実行する AppleScript の文にする（`\` と `"` を逃がす）。
fn admin_script(shell: &str) -> String {
    let escaped = shell.replace('\\', r"\\").replace('"', r#"\""#);
    format!(r#"do shell script "{escaped}" with administrator privileges"#)
}

/// `osascript -e` に渡す、リンクを作る文。`-h` は、行き先がフォルダへのリンクでもたどらずに置き換えるため（BSD の ln）。
pub(crate) fn install_script(target: &Path) -> String {
    admin_script(&format!(
        "mkdir -p /usr/local/bin && ln -sfh {} {LINK_PATH}",
        shell_quote(&target.to_string_lossy())
    ))
}

/// `osascript -e` に渡す、リンクを消す文。リンクのときだけ消す。
pub(crate) fn remove_script() -> String {
    admin_script(&format!("[ -L {LINK_PATH} ] && rm {LINK_PATH}"))
}

/// osascript の結果。
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    Done,
    /// パスワードのダイアログで取り消した（エラーの番号 -128）。
    Canceled,
    Failed(String),
}

pub(crate) fn classify(success: bool, stderr: &str) -> Outcome {
    if success {
        Outcome::Done
    } else if stderr.contains("(-128)") {
        Outcome::Canceled
    } else {
        Outcome::Failed(stderr.trim().to_string())
    }
}

pub(crate) fn install_message(o: &Outcome) -> String {
    match o {
        Outcome::Done => "シェルコマンドをインストールしました。ターミナルで `fude <フォルダ>` と打つと開けます。".into(),
        Outcome::Canceled => "シェルコマンドのインストールを取り消しました。".into(),
        Outcome::Failed(e) => format!("シェルコマンドをインストールできません: {e}"),
    }
}

pub(crate) fn remove_message(o: &Outcome) -> String {
    match o {
        Outcome::Done => "シェルコマンドを削除しました。".into(),
        Outcome::Canceled => "シェルコマンドの削除を取り消しました。".into(),
        Outcome::Failed(e) => format!("シェルコマンドを削除できません: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_only() {
        let exe = Path::new("/Applications/fude.app/Contents/MacOS/fude");
        assert_eq!(bundle_executable(exe), Some(exe.to_path_buf()));
        assert_eq!(bundle_executable(Path::new("/Users/a/fude/target-dev/debug/fude")), None);
        assert_eq!(bundle_executable(Path::new("/x/Contents/MacOS/fude")), None);
    }

    #[test]
    fn install_script_quotes() {
        let s = install_script(Path::new("/Applications/My Apps/it's \"x\"/fude.app/Contents/MacOS/fude"));
        assert_eq!(
            s,
            r#"do shell script "mkdir -p /usr/local/bin && ln -sfh '/Applications/My Apps/it'\\''s \"x\"/fude.app/Contents/MacOS/fude' /usr/local/bin/fude" with administrator privileges"#
        );
    }

    #[test]
    fn remove_script_only_symlink() {
        assert_eq!(
            remove_script(),
            r#"do shell script "[ -L /usr/local/bin/fude ] && rm /usr/local/bin/fude" with administrator privileges"#
        );
    }

    #[test]
    fn classify_results() {
        assert_eq!(classify(true, ""), Outcome::Done);
        assert_eq!(classify(false, "0:41: execution error: User canceled. (-128)"), Outcome::Canceled);
        assert_eq!(
            classify(false, "ln: /usr/local/bin/fude: Permission denied (1)\n"),
            Outcome::Failed("ln: /usr/local/bin/fude: Permission denied (1)".into())
        );
    }

    #[test]
    fn messages() {
        assert_eq!(
            install_message(&Outcome::Done),
            "シェルコマンドをインストールしました。ターミナルで `fude <フォルダ>` と打つと開けます。"
        );
        assert_eq!(install_message(&Outcome::Failed("x".into())), "シェルコマンドをインストールできません: x");
        assert_eq!(remove_message(&Outcome::Done), "シェルコマンドを削除しました。");
    }

    #[test]
    fn link_states() {
        let dir = std::env::temp_dir().join(format!("fude-shell-command-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing");
        assert_eq!(link_state(&missing), LinkState::Missing);
        let file = dir.join("file");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(link_state(&file), LinkState::Other);
        #[cfg(unix)]
        {
            let link = dir.join("link");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert_eq!(link_state(&link), LinkState::Symlink(file.clone()));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
