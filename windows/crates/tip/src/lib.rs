//! Seamtype as a Windows input method: a Text Services Framework text input processor (TIP),
//! the counterpart of Sources/Seamtype (the InputMethodKit layer on macOS). All typing logic
//! is seamtype-core's `InputSession`; this crate adapts TSF to it: keys in, composition
//! (underlined marked text) and commits out, the language bar menu, and COM registration.
//!
//! The DLL runs inside every app the user types in, so nothing here may panic across the COM
//! boundary (`guard`), and it holds no text: typed keys never leave the process or reach a log.

#![allow(non_snake_case)]

mod client;
mod display;
mod edit;
mod keys;
mod langbar;
pub mod log;
pub mod register;
pub mod runtime;
mod service;
mod strings;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicPtr, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, HINSTANCE, HMODULE, S_FALSE,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_core::{implement, IUnknown, Interface, Ref, BOOL, GUID, HRESULT};

pub use service::TextService;

/// The COM class of the text service.
pub const CLSID_SEAMTYPE: GUID = GUID::from_u128(0x2ff7b2b1_d7d9_4ea7_a2a5_4cc3edac15d8);
/// Seamtype's input profile (one, under Korean).
pub const GUID_PROFILE: GUID = GUID::from_u128(0xa13e3052_ef12_4ba3_bd09_87ba0c88957f);
/// The display attribute of a word being composed (underlined).
pub const GUID_DISPLAY_ATTRIBUTE_INPUT: GUID = GUID::from_u128(0x9c4808d0_2801_4fd8_8a83_f7b1c87449b4);
/// The pause key (한/영), preserved so it reaches Seamtype in every app.
pub const GUID_PRESERVED_PAUSE: GUID = GUID::from_u128(0xb5695104_180e_4000_94d6_666d91504c6f);
/// Seamtype is a Korean input method (it types Hangul, and English and Japanese with it).
pub const LANGID_KOREAN: u16 = 0x0412;

static MODULE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// This DLL's module handle.
pub fn module() -> HMODULE {
    HMODULE(MODULE.load(Ordering::Relaxed))
}

/// Runs a COM method body, turning a panic into an error instead of unwinding into the app.
pub(crate) fn guard<T>(f: impl FnOnce() -> windows_core::Result<T>) -> windows_core::Result<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(panic) => {
            let what = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            log::write(&format!("panic caught at the COM boundary: {what}"));
            Err(E_FAIL.into())
        }
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(instance: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(instance.0, Ordering::Relaxed);
        unsafe {
            let _ = DisableThreadLibraryCalls(HMODULE(instance.0));
        }
    }
    true.into()
}

#[implement(IClassFactory)]
struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, riid: *const GUID, object: *mut *mut c_void) -> windows_core::Result<()> {
        guard(|| unsafe {
            if object.is_null() {
                return Err(E_POINTER.into());
            }
            *object = std::ptr::null_mut();
            if outer.is_some() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let service: IUnknown = TextService::new().into();
            service.query(riid, object).ok()
        })
    }

    fn LockServer(&self, _lock: BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(clsid: *const GUID, riid: *const GUID, object: *mut *mut c_void) -> HRESULT {
    guard(|| unsafe {
        if object.is_null() || clsid.is_null() {
            return Err(E_POINTER.into());
        }
        *object = std::ptr::null_mut();
        if *clsid != CLSID_SEAMTYPE {
            return Err(CLASS_E_CLASSNOTAVAILABLE.into());
        }
        let factory: IClassFactory = ClassFactory.into();
        factory.query(riid, object).ok()
    })
    .map_or_else(|e| e.code(), |_| HRESULT(0))
}

/// The DLL stays loaded for the life of the process: TSF keeps calling into it, and unloading
/// a Rust DLL with live statics buys nothing.
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[unsafe(no_mangle)]
extern "system" fn DllRegisterServer() -> HRESULT {
    guard(register::register).map_or_else(|e| e.code(), |_| HRESULT(0))
}

#[unsafe(no_mangle)]
extern "system" fn DllUnregisterServer() -> HRESULT {
    guard(register::unregister).map_or_else(|e| e.code(), |_| HRESULT(0))
}
