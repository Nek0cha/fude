//! アプリ内で読むライセンスの文書（段階 10 の設計書 §5「10a 公開の準備」）。
//! LICENSE、NOTICE、licenses/ の同梱物のライセンス、依存クレートの一覧を 1 つの Markdown にまとめる。

/// fude に同梱しているもの（licenses/ の 1 ファイルごと）。
pub struct Bundled {
    pub name: &'static str,
    pub used_for: &'static str,
    pub license: &'static str,
    /// licenses/ の中のファイル名。
    pub file: &'static str,
    pub text: &'static str,
}

macro_rules! bundled {
    ($name:expr, $used_for:expr, $license:expr, $file:literal) => {
        Bundled {
            name: $name,
            used_for: $used_for,
            license: $license,
            file: $file,
            text: include_str!(concat!("../../../licenses/", $file)),
        }
    };
}

/// licenses/ の THIRD-PARTY.md 以外のすべて。
pub const BUNDLED: &[Bundled] = &[
    bundled!("HackGen Console NF", "ターミナルとエディタの書体（改変せずに埋め込み）", "SIL Open Font License 1.1", "HackGen-OFL.txt"),
    bundled!("Material Icon Theme", "ファイルツリーのアイコン", "MIT License", "material-icon-theme-LICENSE"),
    bundled!("Lucide", "画面のアイコン（gpui-kit-assets 経由）", "ISC License", "lucide-ISC.txt"),
    bundled!("Catppuccin", "テーマ catppuccin-mocha・macchiato・frappe の色", "MIT License", "catppuccin-MIT.txt"),
    bundled!("Dracula", "テーマ dracula の色", "MIT License", "dracula-MIT.txt"),
    bundled!("GitHub VS Code Theme", "テーマ github-dark・github-dark-dimmed の色", "MIT License", "github-vscode-theme-MIT.txt"),
    bundled!("One Dark Pro", "テーマ one-dark-pro の色", "MIT License", "one-dark-pro-MIT.txt"),
    bundled!("Tokyo Night", "テーマ tokyo-night・tokyo-night-storm の色", "MIT License", "tokyo-night-MIT.txt"),
];

const LICENSE: &str = include_str!("../../../LICENSE");
const NOTICE: &str = include_str!("../../../NOTICE");
const THIRD_PARTY: &str = include_str!("../../../licenses/THIRD-PARTY.md");

/// 本文をコードブロックで囲う。本文に ``` が含まれても閉じないよう、backtick は 4 つにする。
fn fenced(out: &mut String, text: &str) {
    out.push_str("````text\n");
    out.push_str(&text.trim_end().replace("\r\n", "\n"));
    out.push_str("\n````\n\n");
}

/// 表示用の Markdown 全体。
pub fn document() -> String {
    let mut out = String::with_capacity(LICENSE.len() + NOTICE.len() + THIRD_PARTY.len() + 64 * 1024);
    out.push_str("# fude\n\nfude は Apache License 2.0 で配布しています。同梱しているものと依存しているクレートのライセンスは、下に続きます。\n\n");
    fenced(&mut out, NOTICE);
    out.push_str("# fude のライセンス\n\n");
    fenced(&mut out, LICENSE);
    out.push_str("# 同梱しているもの\n\n");
    for b in BUNDLED {
        out.push_str(&format!("## {}\n\n- 用途：{}\n- ライセンス：{}\n\n", b.name, b.used_for, b.license));
        fenced(&mut out, b.text);
    }
    out.push_str("# 依存クレート\n\n");
    out.push_str(&THIRD_PARTY.replace("\r\n", "\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_license_file_is_listed() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../licenses");
        for e in std::fs::read_dir(dir).unwrap() {
            let name = e.unwrap().file_name().into_string().unwrap();
            if name == "THIRD-PARTY.md" {
                continue;
            }
            assert!(BUNDLED.iter().any(|b| b.file == name), "{name} が BUNDLED に無い");
        }
    }

    #[test]
    fn document_has_every_part() {
        let doc = document();
        assert!(doc.contains("Apache License")); // fude 本体
        assert!(doc.contains("Copyright (c) 2019, Yuko OTAWARA")); // HackGen の著作権表示
        assert!(doc.contains("not affiliated with")); // 商標の注意書き
        assert!(doc.contains("gpui-pre")); // 依存の一覧
        for b in BUNDLED {
            assert!(doc.contains(b.name), "{}", b.name);
            assert!(doc.contains(b.text.replace("\r\n", "\n").trim()), "{} の本文", b.name);
        }
    }

    #[test]
    fn document_has_no_carriage_returns() {
        // core.autocrlf で手元のファイルが CRLF になっても、表示には LF だけを渡す。
        assert!(!document().contains('\r'));
        let mut out = String::new();
        fenced(&mut out, "a\r\nb\r\n");
        assert_eq!(out, "````text\na\nb\n````\n\n");
    }

    #[test]
    fn texts_are_fenced() {
        // 本文に ``` があっても表示が崩れないよう、囲いは本文に出てこない長さにする。
        let doc = document();
        assert_eq!(doc.matches("````").count() % 2, 0);
        assert!(doc.contains("````text"));
    }
}
