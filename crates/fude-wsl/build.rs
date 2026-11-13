//! 版の印 FUDE_BUILD_ID（<パッケージの版>+<ビルドした時刻の UNIX 秒>）を設定する。
//! ソースが同じでもビルドし直せば別の版になり、Windows の fude が置き直す。

use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // 依存する fude-fs と、依存の版（Cargo.lock）が変わったときも版を変える。rerun-if-changed を書くと
    // パッケージの中の変化も見なくなるため、自分の src と build.rs も挙げる（ディレクトリは中まで見る）。
    for path in ["src", "build.rs", "../fude-fs/src", "../fude-fs/Cargo.toml", "../../Cargo.lock"] {
        println!("cargo:rerun-if-changed={path}");
    }
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    println!("cargo:rustc-env=FUDE_BUILD_ID={version}+{secs}");
}
