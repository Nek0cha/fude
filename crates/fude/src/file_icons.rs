//! ファイル名からアイコンを決める対応表と、同じ名前のタブの区別（設計書 §5.5、§4.1.3）。

use gpui_kit::{Img, SharedString, Styled, img, px};

/// ファイル名の完全一致での対応（大文字小文字は区別しない）。拡張子より先に見る。
const BY_NAME: &[(&str, &str)] = &[
    ("Cargo.toml", "rust"),
    ("Cargo.lock", "lock"),
    ("package.json", "nodejs"),
    ("package-lock.json", "lock"),
    ("pnpm-lock.yaml", "pnpm"),
    ("yarn.lock", "yarn"),
    ("tsconfig.json", "tsconfig"),
    ("Dockerfile", "docker"),
    ("docker-compose.yml", "docker"),
    (".gitignore", "git"),
    (".gitattributes", "git"),
    (".gitmodules", "git"),
    (".editorconfig", "editorconfig"),
    ("Makefile", "makefile"),
    ("README.md", "readme"),
    ("LICENSE", "certificate"),
    ("LICENSE.md", "certificate"),
    ("LICENSE.txt", "certificate"),
    ("CLAUDE.md", "markdown"),
    ("AGENTS.md", "markdown"),
    (".env", "tune"),
    ("rust-toolchain.toml", "rust"),
    ("vite.config.ts", "vite"),
    ("go.mod", "go-mod"),
    ("go.sum", "go-mod"),
];

/// 拡張子での対応（小文字で書く）。
const BY_EXT: &[(&str, &str)] = &[
    ("rs", "rust"),
    ("ts", "typescript"),
    ("mts", "typescript"),
    ("cts", "typescript"),
    ("tsx", "react_ts"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("jsx", "react"),
    ("json", "json"),
    ("jsonc", "json"),
    ("toml", "toml"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("md", "markdown"),
    ("markdown", "markdown"),
    ("py", "python"),
    ("go", "go"),
    ("c", "c"),
    ("h", "c"),
    ("cpp", "cpp"),
    ("cc", "cpp"),
    ("cxx", "cpp"),
    ("hpp", "cpp"),
    ("hh", "cpp"),
    ("cs", "csharp"),
    ("java", "java"),
    ("kt", "kotlin"),
    ("kts", "kotlin"),
    ("swift", "swift"),
    ("rb", "ruby"),
    ("php", "php"),
    ("lua", "lua"),
    ("sh", "console"),
    ("bash", "console"),
    ("zsh", "console"),
    ("fish", "console"),
    ("ps1", "powershell"),
    ("psm1", "powershell"),
    ("bat", "console"),
    ("cmd", "console"),
    ("html", "html"),
    ("htm", "html"),
    ("css", "css"),
    ("scss", "sass"),
    ("sass", "sass"),
    ("vue", "vue"),
    ("svelte", "svelte"),
    ("sql", "database"),
    ("xml", "xml"),
    ("svg", "svg"),
    ("png", "image"),
    ("jpg", "image"),
    ("jpeg", "image"),
    ("gif", "image"),
    ("webp", "image"),
    ("ico", "image"),
    ("bmp", "image"),
    ("pdf", "pdf"),
    ("zip", "zip"),
    ("tar", "zip"),
    ("gz", "zip"),
    ("tgz", "zip"),
    ("7z", "zip"),
    ("lock", "lock"),
    ("txt", "document"),
    ("log", "log"),
    ("csv", "table"),
    ("env", "tune"),
    ("wasm", "webassembly"),
    ("dart", "dart"),
    ("zig", "zig"),
    ("nix", "nix"),
    ("proto", "proto"),
    ("graphql", "graphql"),
    ("gql", "graphql"),
    ("tf", "terraform"),
    ("ini", "settings"),
    ("cfg", "settings"),
    ("conf", "settings"),
    ("diff", "diff"),
    ("patch", "diff"),
];

#[cfg_attr(not(test), allow(dead_code))]
/// 同梱するアイコンの名前の全部（`assets/file-icons/{name}.svg`）。
pub(crate) const ICONS: &[&str] = &[
    "c",
    "certificate",
    "console",
    "cpp",
    "csharp",
    "css",
    "dart",
    "database",
    "diff",
    "docker",
    "document",
    "editorconfig",
    "file",
    "folder",
    "folder-open",
    "git",
    "go",
    "go-mod",
    "graphql",
    "html",
    "image",
    "java",
    "javascript",
    "json",
    "kotlin",
    "lock",
    "log",
    "lua",
    "makefile",
    "markdown",
    "nix",
    "nodejs",
    "pdf",
    "php",
    "pnpm",
    "powershell",
    "proto",
    "python",
    "react",
    "react_ts",
    "readme",
    "ruby",
    "rust",
    "sass",
    "settings",
    "svelte",
    "svg",
    "swift",
    "table",
    "terraform",
    "toml",
    "tsconfig",
    "tune",
    "typescript",
    "vite",
    "vue",
    "webassembly",
    "xml",
    "yaml",
    "yarn",
    "zig",
    "zip",
];

/// ファイル名（パスではない）からアイコンの名前を決める。対応が無ければ "file"。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn icon_for_file(name: &str) -> &'static str {
    if let Some((_, icon)) = BY_NAME.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return icon;
    }
    // 先頭のドットだけのもの（.bashrc）は拡張子として扱わない。
    let ext = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => return "file",
    };
    BY_EXT
        .iter()
        .find(|(e, _)| *e == ext)
        .map_or("file", |(_, icon)| icon)
}

#[allow(dead_code)] // Task 11 で使う。
pub(crate) fn icon_for_folder(open: bool) -> &'static str {
    if open { "folder-open" } else { "folder" }
}

/// アセットの中のアイコンのパス。
pub(crate) fn icon_path(icon: &str) -> SharedString {
    format!("file-icons/{icon}.svg").into()
}

/// ファイルアイコンを 14px 四方で描く。色は少し暗くする（不透明度 0.8 で、背景に 20% 寄せる）。
pub(crate) fn file_icon(icon: &'static str) -> Img {
    img(icon_path(icon)).size(px(14.)).flex_shrink_0().opacity(0.8)
}

/// タブに出す (ファイル名, 親フォルダ名)。親フォルダ名は、同じファイル名が 2 つ以上あるときだけ付ける。
/// ルート直下のファイルの親は "/" と出す。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn tab_labels(rels: &[&str]) -> Vec<(String, Option<String>)> {
    let split = |rel: &str| -> (String, String) {
        match rel.rsplit_once('/') {
            Some((dir, name)) => (
                name.to_string(),
                dir.rsplit('/').next().unwrap_or(dir).to_string(),
            ),
            None => (rel.to_string(), "/".to_string()),
        }
    };
    let parts: Vec<(String, String)> = rels.iter().map(|r| split(r)).collect();
    parts
        .iter()
        .map(|(name, parent)| {
            let dup = parts.iter().filter(|(n, _)| n == name).count() > 1;
            (name.clone(), dup.then(|| parent.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_by_name_then_extension() {
        assert_eq!(icon_for_file("Cargo.toml"), "rust");
        assert_eq!(icon_for_file("other.toml"), "toml");
        assert_eq!(icon_for_file("main.RS"), "rust");
        assert_eq!(icon_for_file("App.tsx"), "react_ts");
        assert_eq!(icon_for_file("Dockerfile"), "docker");
        assert_eq!(icon_for_file("noext"), "file");
        assert_eq!(icon_for_file(".bashrc"), "file");
    }

    #[test]
    fn labels_add_parent_for_duplicates() {
        let l = tab_labels(&["src/a/mod.rs", "src/b/mod.rs", "main.rs"]);
        assert_eq!(l[0], ("mod.rs".to_string(), Some("a".to_string())));
        assert_eq!(l[1], ("mod.rs".to_string(), Some("b".to_string())));
        assert_eq!(l[2], ("main.rs".to_string(), None));
        // ルート直下と重なったときは、ルートを "/" と出す。
        let l = tab_labels(&["mod.rs", "x/mod.rs"]);
        assert_eq!(l[0].1.as_deref(), Some("/"));
    }

    #[test]
    fn icons_list_covers_every_mapping() {
        for (_, icon) in BY_NAME.iter().chain(BY_EXT) {
            assert!(ICONS.contains(icon), "{icon} が ICONS に無い");
        }
        for icon in ["file", "folder", "folder-open"] {
            assert!(ICONS.contains(&icon));
        }
    }
}
