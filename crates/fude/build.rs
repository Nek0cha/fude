//! Windows では、WSL の中でビルドした Linux 用の fude（target/linux/fude-linux）と版の印を埋め込む。
//! `$OUT_DIR/linux_bin.rs` に `LINUX_BIN: Option<(&[u8], &str)>` を書く。無ければ None にして警告を出す。
//! また、`assets/file-icons` の SVG を `$OUT_DIR/file_icons.rs` の `FILE_ICONS: &[(&str, &[u8])]` に埋め込む。

use std::path::PathBuf;

fn main() {
    embed_file_icons();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("linux_bin.rs");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let linux = manifest.join("..").join("..").join("target").join("linux");
    let bin = linux.join("fude-linux");
    let id_file = linux.join("fude-linux.id");
    println!("cargo:rerun-if-changed={}", bin.display());
    println!("cargo:rerun-if-changed={}", id_file.display());

    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let id = std::fs::read_to_string(&id_file).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let code = match (windows, id) {
        (true, Some(id)) if bin.is_file() => {
            let bin = bin.canonicalize().unwrap();
            format!("pub const LINUX_BIN: Option<(&[u8], &str)> = Some((include_bytes!({:?}), {id:?}));\n", bin)
        }
        (true, _) => {
            println!("cargo:warning=target/linux/fude-linux が無い（WSL の中で scripts/build-linux.sh を実行する）");
            "pub const LINUX_BIN: Option<(&[u8], &str)> = None;\n".to_string()
        }
        (false, _) => "pub const LINUX_BIN: Option<(&[u8], &str)> = None;\n".to_string(),
    };
    std::fs::write(out, code).unwrap();
}

/// assets/file-icons/*.svg を (ファイル名, 中身) の表にする。
fn embed_file_icons() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dir = manifest.join("..").join("..").join("assets").join("file-icons").canonicalize().unwrap();
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".svg"))
        .collect();
    names.sort();
    let mut code = String::from("pub const FILE_ICONS: &[(&str, &[u8])] = &[
");
    for name in names {
        code.push_str(&format!("    ({name:?}, include_bytes!({:?})),
", dir.join(&name)));
    }
    code.push_str("];
");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("file_icons.rs");
    std::fs::write(out, code).unwrap();
}
