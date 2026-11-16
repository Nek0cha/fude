//! Windows では、WSL の中でビルドした Linux 用の fude（target/linux/fude-linux）と版の印を埋め込む。
//! `$OUT_DIR/linux_bin.rs` に `LINUX_BIN: Option<(&[u8], &str)>` を書く。無ければ None にして警告を出す。
//! Windows では exe にアプリのアイコン（リソースの番号 1。GPUI が窓のアイコンに使う）と版の情報も埋め込む。
//! また、`assets/file-icons` の SVG を `$OUT_DIR/file_icons.rs` の `FILE_ICONS: &[(&str, &[u8])]` に埋め込む。

use std::path::PathBuf;

fn main() {
    embed_file_icons();
    embed_windows_resource();
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
            warn_if_stale(&bin, &manifest);
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

/// fude-linux の元のソース（fude-wsl と、それが使う fude-core、fude-fs）。
const LINUX_SOURCES: [&str; 3] = ["fude-wsl", "fude-core", "fude-fs"];

/// fude-linux がソースより古ければ警告する。古いまま埋め込むと、WSL に置く fude が新しい機能
/// （fude hook など）を知らず、版の印も変わらないため置き直されない。
fn warn_if_stale(bin: &std::path::Path, manifest: &std::path::Path) {
    let Ok(built) = std::fs::metadata(bin).and_then(|m| m.modified()) else { return };
    for name in LINUX_SOURCES {
        let src = manifest.join("..").join(name).join("src");
        println!("cargo:rerun-if-changed={}", src.display());
        if newest_mtime(&src).is_some_and(|t| t > built) {
            println!("cargo:warning=target/linux/fude-linux が crates/{name} より古い（WSL の中で scripts/build-linux.sh を実行する）");
            return;
        }
    }
}

fn newest_mtime(dir: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if meta.is_dir() { newest_mtime(&e.path()) } else { meta.modified().ok() }
        })
        .max()
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

/// Windows のターゲットのとき、assets/icon/fude.ico と版の情報を exe のリソースに埋め込む（10b の設計書 §5.2）。
fn embed_windows_resource() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("..").join("..").join("assets").join("icon").join("fude.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let mut res = winresource::WindowsResource::new();
    // GPUI は窓のアイコンを番号 1 から読む（gpui-pre-windows の platform.rs の load_icon）。
    res.set_icon_with_id(icon.to_str().unwrap(), "1");
    res.set("ProductName", "fude");
    res.set("FileDescription", "fude");
    res.set("LegalCopyright", "Copyright 2026 Nek0cha");
    res.compile().expect("exe のリソースを埋め込めない");
}
