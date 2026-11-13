//! 本物の WSL を使う結合テスト（Windows だけ）。
//! deploy_and_serve_on_real_wsl は FUDE_TEST_DISTRO と、WSL の中で scripts/build-linux.sh を実行して作った
//! target/linux/fude-linux が要るため #[ignore] にしてある。

#![cfg(windows)]

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fude_fs::wsl::{WslConnector, default_bin, deploy, installed_version};
use fude_fs::{ProjectFs, RelPath, RemoteClient, RemoteFs};

#[test]
fn missing_distro_reports_error() {
    let err = installed_version("NoSuchDistro-xyz", "~/.local/bin/fude").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound, "{err}");
}

#[test]
fn default_bin_rules() {
    assert_eq!(default_bin(&|_| None), "~/.local/bin/fude");
    assert_eq!(default_bin(&|k| (k == "FUDE_WSL_BIN_DIR").then(|| "/tmp/x".to_string())), "/tmp/x/fude");
}

#[test]
#[ignore]
fn deploy_and_serve_on_real_wsl() {
    let distro = std::env::var("FUDE_TEST_DISTRO").expect("FUDE_TEST_DISTRO を設定する");
    let linux = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/linux");
    let bytes = std::fs::read(linux.join("fude-linux")).expect("WSL の中で scripts/build-linux.sh を実行する");
    let id = std::fs::read_to_string(linux.join("fude-linux.id")).unwrap().trim().to_string();
    let dir = format!("/tmp/fude-test-{}", std::process::id());
    let bin = format!("{dir}/fude");

    assert_eq!(installed_version(&distro, &bin).unwrap(), None);
    deploy(&distro, &bin, &bytes, &std::env::current_exe().unwrap()).unwrap();
    assert_eq!(installed_version(&distro, &bin).unwrap(), Some(id));

    let connector = WslConnector { distro: distro.clone(), bin: bin.clone() };
    let client = Arc::new(RemoteClient::new(Box::new(connector), Duration::from_secs(30)));
    let fs = RemoteFs::new(client, format!("{dir}/work"));
    let path = RelPath::new("a/作業 b.txt").unwrap();
    fs.write(&path, b"hello\n\0").unwrap();
    assert_eq!(fs.read(&path).unwrap(), b"hello\n\0");
    assert_eq!(fs.list_dir(&RelPath::new("a").unwrap()).unwrap()[0].name, "作業 b.txt");

    let host_exe = std::process::Command::new("wsl.exe")
        .args(["-d", &distro, "-e", "cat", &format!("{dir}/host-exe")])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&host_exe.stdout).starts_with("/mnt/"));
    let _ = std::process::Command::new("wsl.exe").args(["-d", &distro, "-e", "rm", "-rf", &dir]).status();
}
