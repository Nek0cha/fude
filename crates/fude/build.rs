//! Windows では、WSL の中でビルドした Linux 用の fude（target/linux/fude-linux）と版の印を埋め込む。
//! `$OUT_DIR/linux_bin.rs` に `LINUX_BIN: Option<(&[u8], &str)>` を書く。無ければ None にして警告を出す。

use std::path::PathBuf;

fn main() {
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
