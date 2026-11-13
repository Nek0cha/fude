//! プロジェクトのファイルへのアクセス（ProjectFs）を用意する。WSL のプロジェクトは、ディストリビューションごとに
//! 1 つの fude serve と話す。最初に使うときに WSL に置いた fude の版を確かめ、違えば置き直す。

use std::io;
use std::sync::Arc;

use fude_core::workspace::ProjectRoot;
use fude_fs::{LocalFs, ProjectFs};

/// LINUX_BIN: Option<(&[u8], &str)>。WSL に置く Linux 用の fude とその版の印（build.rs が書く。Windows 以外は None）。
#[cfg_attr(not(windows), allow(dead_code))]
mod bundled {
    include!(concat!(env!("OUT_DIR"), "/linux_bin.rs"));
}

/// 置いてある版と同梱の版を比べ、違う（または無い）なら置く必要がある。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn needs_deploy(installed: Option<&str>, bundled: &str) -> bool {
    installed != Some(bundled)
}

/// ディストリビューションごとに 1 つの RemoteClient を持つ。最初に使うときに版を確かめ、違えば置き直す。
/// 確かめに失敗したら覚えず、次に使うときにやり直す。
pub(crate) struct WslHosts {
    #[cfg(windows)]
    /// ディストリビューション（小文字）ごとの枠。版の確認と配置は枠の中だけを止め、ほかのディストリビューションを待たせない。
    clients: std::sync::Mutex<std::collections::HashMap<String, Arc<std::sync::Mutex<Option<Arc<fude_fs::RemoteClient>>>>>>,
    /// WSL 側の置き場所（FUDE_WSL_BIN_DIR で変えられる）。
    #[cfg(windows)]
    bin: String,
}

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use fude_fs::RemoteClient;
    use fude_fs::wsl::{WslConnector, default_bin, deploy, installed_version};

    use super::*;

    /// 接続して最初の要求は WSL の起動を待つため長く、それ以外は短く待つ。
    const FIRST_TIMEOUT: Duration = Duration::from_secs(30);
    const TIMEOUT: Duration = Duration::from_secs(10);

    impl WslHosts {
        pub fn new() -> WslHosts {
            WslHosts { clients: Default::default(), bin: default_bin(&|k| std::env::var(k).ok()) }
        }

        /// 裏のスレッドから呼ぶ（WSL の起動を待つことがある）。LINUX_BIN が None なら Err（同梱されていない）。
        pub fn client(&self, distro: &str) -> io::Result<Arc<RemoteClient>> {
            let slot = {
                let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
                clients.entry(distro.to_ascii_lowercase()).or_default().clone()
            };
            let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(client) = slot.as_ref() {
                return Ok(client.clone());
            }
            let (bytes, bundled) = bundled::LINUX_BIN.ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "Linux 用の fude が同梱されていない（scripts/build-linux.sh）")
            })?;
            let installed = installed_version(distro, &self.bin)?;
            if needs_deploy(installed.as_deref(), bundled) {
                deploy(distro, &self.bin, bytes, &std::env::current_exe()?)?;
            }
            let connector = WslConnector { distro: distro.to_string(), bin: self.bin.clone() };
            let client = Arc::new(RemoteClient::new(Box::new(connector), TIMEOUT).with_first_timeout(FIRST_TIMEOUT));
            *slot = Some(client.clone());
            Ok(client)
        }
    }
}

#[cfg(not(windows))]
impl WslHosts {
    pub fn new() -> WslHosts {
        WslHosts {}
    }
}

/// プロジェクトの ProjectFs。Local は LocalFs、Wsl は WslHosts の client から RemoteFs（Windows だけ）。
/// Wsl はディストリビューションの起動や配置を待つことがあるため、裏のスレッドから呼ぶ。
pub(crate) fn project_fs(root: &ProjectRoot, hosts: &WslHosts) -> io::Result<Arc<dyn ProjectFs>> {
    match root {
        ProjectRoot::Local(path) => Ok(Arc::new(LocalFs::new(path.clone()))),
        #[cfg(windows)]
        ProjectRoot::Wsl { distro, path } => {
            Ok(Arc::new(fude_fs::RemoteFs::new(hosts.client(distro)?, path.clone())))
        }
        #[cfg(not(windows))]
        ProjectRoot::Wsl { .. } => {
            let _ = hosts;
            Err(io::Error::new(io::ErrorKind::Unsupported, "WSL のプロジェクトは Windows だけで開ける"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_deploy_rules() {
        assert!(needs_deploy(None, "a"));
        assert!(!needs_deploy(Some("a"), "a"));
        assert!(needs_deploy(Some("a"), "b"));
    }

    #[test]
    fn local_project_fs_reads_branch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let fs = project_fs(&ProjectRoot::Local(dir.path().to_path_buf()), &WslHosts::new()).unwrap();
        assert_eq!(fs.branch().unwrap(), Some("main".to_string()));
    }
}
