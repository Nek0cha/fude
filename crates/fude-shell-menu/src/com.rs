//! COM の部分：`IExplorerCommand` の「fude で開く」と、そのクラスファクトリー。

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::time::Duration;

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_NOTIMPL, E_POINTER, HMODULE, S_FALSE,
};
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW,
};
use windows::Win32::UI::Shell::{
    ECF_DEFAULT, ECS_ENABLED, IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray,
    SHStrDupW, SIGDN_FILESYSPATH,
};
use windows_core::{BOOL, GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement, w};

use crate::launch::{launch_command, parse_fude_path};

/// マニフェスト（`installer/context-menu/AppxManifest.xml`）の CLSID と同じ値にする。
pub const FUDE_MENU_CLSID: GUID = GUID::from_u128(0x219640c0_0705_4f75_bdea_75d9574aa170);

/// 複数の項目を開くときの間隔。fude が動いていないとき、最初の fude.exe が先に立ち上がり、
/// 残りが IPC で渡せるようにする。
const LAUNCH_GAP: Duration = Duration::from_millis(500);

/// COM の入口で panic をエクスプローラー（dllhost）に漏らさず、E_FAIL にする。
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(E_FAIL.into()))
}

/// この DLL のフォルダにある `fude-path.txt` から fude.exe のパスを読む（毎回読む）。
fn fude_exe() -> Option<PathBuf> {
    let mut module = HMODULE::default();
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    let anchor = PCWSTR(fude_exe as *const () as *const u16);
    unsafe { GetModuleHandleExW(flags, anchor, &mut module) }.ok()?;
    let mut buf = vec![0u16; 32768];
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    let dll = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
    let text = std::fs::read_to_string(dll.parent()?.join("fude-path.txt")).ok()?;
    parse_fude_path(&text).filter(|p| p.is_file())
}

/// 選ばれた項目のファイルシステムのパス。パスを持たない項目（「PC」など）は飛ばす。
fn item_paths(items: &IShellItemArray) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for i in 0..unsafe { items.GetCount()? } {
        let item = unsafe { items.GetItemAt(i)? };
        let Ok(name) = (unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }) else { continue };
        let path = unsafe { name.to_string() };
        unsafe { CoTaskMemFree(Some(name.0 as *const c_void)) };
        if let Ok(path) = path {
            paths.push(PathBuf::from(path));
        }
    }
    Ok(paths)
}

#[implement(IExplorerCommand)]
struct OpenInFude;

impl IExplorerCommand_Impl for OpenInFude_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        unsafe { SHStrDupW(w!("fude で開く")) }
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        guard(|| {
            let exe = fude_exe().ok_or(windows_core::Error::from(E_NOTIMPL))?;
            let icon = HSTRING::from(format!("{},0", exe.display()));
            unsafe { SHStrDupW(&icon) }
        })
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(FUDE_MENU_CLSID)
    }

    fn GetState(&self, _items: Ref<IShellItemArray>, _ok_to_be_slow: BOOL) -> Result<u32> {
        Ok(ECS_ENABLED.0 as u32)
    }

    /// 項目ごとに `fude.exe <パス>` を起動する。fude.exe が無い、起動できないときは何もしない。
    fn Invoke(&self, items: Ref<IShellItemArray>, _bind: Ref<IBindCtx>) -> Result<()> {
        guard(|| {
            let Some(exe) = fude_exe() else { return Ok(()) };
            let Some(items) = items.as_ref() else { return Ok(()) };
            for (i, path) in item_paths(items)?.iter().enumerate() {
                if i > 0 {
                    std::thread::sleep(LAUNCH_GAP);
                }
                let _ = launch_command(&exe, path).spawn();
            }
            Ok(())
        })
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, riid: *const GUID, object: *mut *mut c_void) -> Result<()> {
        if object.is_null() || riid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *object = core::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        guard(|| {
            let command: IExplorerCommand = OpenInFude.into();
            unsafe { command.query(riid, object).ok() }
        })
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM が DLL からクラスファクトリーを取り出す入口。
#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(clsid: *const GUID, riid: *const GUID, object: *mut *mut c_void) -> HRESULT {
    if clsid.is_null() || riid.is_null() || object.is_null() {
        return E_POINTER;
    }
    unsafe { *object = core::ptr::null_mut() };
    if unsafe { *clsid } != FUDE_MENU_CLSID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    catch_unwind(|| {
        let factory: IClassFactory = Factory.into();
        unsafe { factory.query(riid, object) }
    })
    .unwrap_or(E_FAIL)
}

/// 代理のプロセス（dllhost）が DLL を外してよいか。外さずに置いておく。
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
