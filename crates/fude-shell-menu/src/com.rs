//! COM の部分：`IExplorerCommand` の「fude で開く」と、そのクラスファクトリー。

use core::ffi::c_void;

use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOTIMPL, E_POINTER, S_FALSE};
use windows::Win32::System::Com::{IBindCtx, IClassFactory, IClassFactory_Impl};
use windows::Win32::UI::Shell::{
    ECF_DEFAULT, ECS_ENABLED, IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray, SHStrDupW,
};
use windows_core::{BOOL, GUID, HRESULT, IUnknown, Interface, PWSTR, Ref, Result, implement, w};

/// マニフェスト（`installer/context-menu/AppxManifest.xml`）の CLSID と同じ値にする。
pub const FUDE_MENU_CLSID: GUID = GUID::from_u128(0x219640c0_0705_4f75_bdea_75d9574aa170);

#[implement(IExplorerCommand)]
struct OpenInFude;

impl IExplorerCommand_Impl for OpenInFude_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        unsafe { SHStrDupW(w!("fude で開く")) }
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
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

    fn Invoke(&self, _items: Ref<IShellItemArray>, _bind: Ref<IBindCtx>) -> Result<()> {
        Ok(())
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
        let command: IExplorerCommand = OpenInFude.into();
        unsafe { command.query(riid, object).ok() }
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
    let factory: IClassFactory = Factory.into();
    unsafe { factory.query(riid, object) }
}

/// 代理のプロセス（dllhost）が DLL を外してよいか。外さずに置いておく。
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
