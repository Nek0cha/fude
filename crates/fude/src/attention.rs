//! 返事待ちを OS に知らせる（設計書 §5.8、§7）。Windows はタスクバーのボタンのオーバーレイと点滅、
//! macOS は Dock のバッジと跳ねる動き。トーストと通知センターは、インストールされたアプリが要るため段階 10 で行う。

use gpui_kit::Window;

/// Dock のバッジの文字。0 は None、100 以上は "99+"。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn badge_label(count: usize) -> Option<String> {
    match count {
        0 => None,
        1..=99 => Some(count.to_string()),
        _ => Some("99+".to_string()),
    }
}

/// 新しく返事待ちになり、ウィンドウが前面でないときだけ知らせる。
pub(crate) fn should_request_attention(became_waiting: bool, window_active: bool) -> bool {
    became_waiting && !window_active
}

/// 返事待ちの数をタスクバー（オーバーレイの琥珀色の点）／Dock（数）に出す。0 で消す。ほかの OS では何もしない。
pub(crate) fn set_badge(window: &Window, count: usize) {
    #[cfg(windows)]
    win::set_overlay(window, count > 0);
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        mac::set_badge(badge_label(count));
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = (window, count);
}

/// Windows は FlashWindowEx（FLASHW_TRAY | FLASHW_TIMERNOFG）、macOS は requestUserAttention(Informational)。
pub(crate) fn request_attention(window: &Window) {
    #[cfg(windows)]
    win::flash(window);
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        mac::bounce();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = window;
}

/// 琥珀色の点（直径 size の円、縁はなめらかに）の、上から下への BGRA（アルファを掛けた値）。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn dot_pixels(size: u32, rgb: (u8, u8, u8)) -> Vec<u32> {
    let r = size as f32 / 2.0;
    let mut out = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - r;
            let dy = y as f32 + 0.5 - r;
            let a = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            let mul = |c: u8| (c as f32 * a).round() as u32;
            let alpha = (a * 255.0).round() as u32;
            out.push((alpha << 24) | (mul(rgb.0) << 16) | (mul(rgb.1) << 8) | mul(rgb.2));
        }
    }
    out
}

#[cfg(windows)]
mod win {
    use std::cell::RefCell;

    use gpui_kit::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{
        FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx, HICON, ICONINFO, CreateIconIndirect,
    };
    use windows::core::w;

    /// 点の大きさ（タスクバーのオーバーレイは 16×16）。
    const DOT: u32 = 16;
    /// `warn`（#ffd28a）。
    const AMBER: (u8, u8, u8) = (0xff, 0xd2, 0x8a);

    thread_local! {
        /// 作ったら使い回す（UI のスレッドだけで使う）。
        static TASKBAR: RefCell<Option<(ITaskbarList3, HICON)>> = const { RefCell::new(None) };
    }

    fn hwnd(window: &Window) -> Option<HWND> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut core::ffi::c_void)),
            _ => None,
        }
    }

    fn dot_icon() -> windows::core::Result<HICON> {
        let pixels = super::dot_pixels(DOT, AMBER);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: DOT as i32,
                // 負の高さで、上から下の並びにする。
                biHeight: -(DOT as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: bits は CreateDIBSection が確保した DOT×DOT×4 バイトを指し、その範囲だけに書く。
        // 作った GDI のオブジェクトは、アイコンに写したあとで消す。
        unsafe {
            let mut bits = std::ptr::null_mut();
            let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());
            let mask = CreateBitmap(DOT as i32, DOT as i32, 1, 1, None);
            let icon = CreateIconIndirect(&ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() });
            let _ = DeleteObject(color.into());
            let _ = DeleteObject(mask.into());
            icon
        }
    }

    fn taskbar() -> windows::core::Result<(ITaskbarList3, HICON)> {
        // SAFETY: UI のスレッドで呼ぶ。COM が初期化済みなら CoInitializeEx は失敗の値を返すだけで害は無い。
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let list: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)?;
            list.HrInit()?;
            Ok((list, dot_icon()?))
        }
    }

    pub(super) fn set_overlay(window: &Window, on: bool) {
        let Some(hwnd) = hwnd(window) else { return };
        TASKBAR.with(|cell| {
            let mut cell = cell.borrow_mut();
            if cell.is_none() {
                match taskbar() {
                    Ok(t) => *cell = Some(t),
                    Err(e) => {
                        eprintln!("タスクバーのバッジを出せない: {e}");
                        return;
                    }
                }
            }
            let Some((list, icon)) = cell.as_ref() else { return };
            // SAFETY: hwnd はこのウィンドウのもの、icon は作ったまま持っているもの。
            let result = unsafe {
                if on { list.SetOverlayIcon(hwnd, *icon, w!("返事待ち")) } else { list.SetOverlayIcon(hwnd, HICON::default(), None) }
            };
            if let Err(e) = result {
                eprintln!("タスクバーのバッジを出せない: {e}");
            }
        });
    }

    pub(super) fn flash(window: &Window) {
        let Some(hwnd) = hwnd(window) else { return };
        let info = FLASHWINFO {
            cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd,
            dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
            uCount: 0,
            dwTimeout: 0,
        };
        // SAFETY: info はこの関数の中で有効。
        unsafe {
            let _ = FlashWindowEx(&info);
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSRequestUserAttentionType};
    use objc2_foundation::NSString;

    pub(super) fn set_badge(label: Option<String>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        let label = label.map(|s| NSString::from_str(&s));
        app.dockTile().setBadgeLabel(label.as_deref());
    }

    pub(super) fn bounce() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        app.requestUserAttention(NSRequestUserAttentionType::InformationalRequest);
    }
}

#[cfg(test)]
mod tests {
    use super::{badge_label, should_request_attention};

    #[test]
    fn badge_labels() {
        assert_eq!(badge_label(0), None);
        assert_eq!(badge_label(3).as_deref(), Some("3"));
        assert_eq!(badge_label(120).as_deref(), Some("99+"));
    }

    #[test]
    fn attention_only_when_new_and_inactive() {
        assert!(should_request_attention(true, false));
        assert!(!should_request_attention(true, true));
        assert!(!should_request_attention(false, false));
    }
}
