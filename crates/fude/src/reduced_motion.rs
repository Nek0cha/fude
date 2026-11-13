//! OS の「動きを減らす」設定（設計書 §6.3）。Windows は「アニメーション効果」がオフ、macOS は「視差効果を減らす」がオン。

/// 動きを減らすか。読めなければ false（動きを付ける）。
pub(crate) fn reduced_motion() -> bool {
    imp()
}

#[cfg(windows)]
fn imp() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
    let mut enabled: i32 = 1;
    // SAFETY: 出力先は BOOL 1 つで、大きさの引数は使われない。
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut enabled as *mut i32).cast(), 0) };
    ok != 0 && enabled == 0
}

#[cfg(target_os = "macos")]
fn imp() -> bool {
    // NSWorkspace.accessibilityDisplayShouldReduceMotion と同じ値。Objective-C の依存を足さないため、設定を読む。
    std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
}

#[cfg(not(any(windows, target_os = "macos")))]
fn imp() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn reading_does_not_panic() {
        let _ = super::reduced_motion();
    }
}
