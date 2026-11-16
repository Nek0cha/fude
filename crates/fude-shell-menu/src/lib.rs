//! Windows 11 のエクスプローラーの最初の右クリックメニューに「fude で開く」を出す DLL（10f）。
//! `scripts/install-context-menu.ps1` が sparse package で登録する。Windows 以外では中身が無い。

pub mod launch;

#[cfg(windows)]
mod com;
