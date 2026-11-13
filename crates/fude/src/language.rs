//! ファイル名から、GPUI Kit のハイライトの言語名を決める。

/// rel（"/" 区切り）のファイル名と拡張子から、gpui-component の登録名。わからなければ None（ハイライトなし）。
/// ここに挙げた言語は、Cargo.toml の gpui-kit の tree-sitter の feature と揃える。
pub fn language_for(rel: &str) -> Option<&'static str> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name {
        "Makefile" | "makefile" | "GNUmakefile" => return Some("make"),
        _ => {}
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "json" | "jsonc" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "py" => "python",
        "go" => "go",
        "sh" | "bash" | "zsh" | "fish" => "bash",
        "css" => "css",
        "html" | "htm" => "html",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "sql" => "sql",
        "lua" => "lua",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::language_for;

    #[test]
    fn language_for_common_files() {
        assert_eq!(language_for("src/main.rs"), Some("rust"));
        assert_eq!(language_for("a.TSX"), Some("tsx"));
        assert_eq!(language_for("web/app.mjs"), Some("javascript"));
        assert_eq!(language_for("Cargo.toml"), Some("toml"));
        assert_eq!(language_for(".github/ci.yml"), Some("yaml"));
        assert_eq!(language_for("docs/a.md"), Some("markdown"));
        assert_eq!(language_for("config.fish"), Some("bash"));
        assert_eq!(language_for("x.hpp"), Some("cpp"));
        assert_eq!(language_for("Makefile"), Some("make"));
        assert_eq!(language_for("README"), None);
        assert_eq!(language_for("a.unknown"), None);
    }
}
