//! Diagnostics for the log (only while it is on): who called into the text service, and what
//! IME messages an IMM32 app's windows get. Code addresses and message kinds only, never text.

use std::cell::RefCell;
use std::ffi::c_void;

use windows::Win32::Foundation::{HMODULE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace;
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, CWPSTRUCT, HHOOK, MSG, WH_CALLWNDPROC, WH_GETMESSAGE,
};
use windows_core::PCWSTR;

/// The module and offset of a code address, with the nearest exported function below it.
fn describe(address: *mut c_void) -> String {
    unsafe {
        let mut module = HMODULE::default();
        let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        if GetModuleHandleExW(flags, PCWSTR(address as *const u16), &mut module).is_err() {
            return format!("{address:?}");
        }
        let mut name = [0u16; 260];
        let n = GetModuleFileNameW(Some(module), &mut name) as usize;
        let path = String::from_utf16_lossy(&name[..n]);
        let file = path.rsplit('\\').next().unwrap_or(&path).to_string();
        let base = module.0 as usize;
        let rva = (address as usize).wrapping_sub(base) as u32;
        match nearest_export(base as *const u8, rva) {
            Some((symbol, at)) => format!("{file}!{symbol}+{:#x}", rva - at),
            None => format!("{file}+{rva:#x}"),
        }
    }
}

/// The exported function of the image at `base` that starts nearest below `rva`.
unsafe fn nearest_export(base: *const u8, rva: u32) -> Option<(String, u32)> {
    unsafe {
        let read_u16 = |at: usize| (base.add(at) as *const u16).read_unaligned();
        let read_u32 = |at: usize| (base.add(at) as *const u32).read_unaligned();
        if read_u16(0) != 0x5A4D {
            return None;
        }
        let nt = read_u32(0x3C) as usize;
        let magic = read_u16(nt + 24);
        let directories = nt + 24 + if magic == 0x20B { 112 } else { 96 };
        let export = read_u32(directories) as usize;
        if export == 0 {
            return None;
        }
        let functions = read_u32(export + 28) as usize;
        let names = read_u32(export + 32) as usize;
        let ordinals = read_u32(export + 36) as usize;
        let count = read_u32(export + 24) as usize;
        let mut best: Option<(usize, u32)> = None;
        for i in 0..count {
            let ordinal = read_u16(ordinals + i * 2) as usize;
            let start = read_u32(functions + ordinal * 4);
            if start <= rva && best.map(|(_, b)| start > b).unwrap_or(true) {
                best = Some((i, start));
            }
        }
        let (i, start) = best?;
        let name_at = read_u32(names + i * 4) as usize;
        let mut end = name_at;
        while *base.add(end) != 0 && end - name_at < 400 {
            end += 1;
        }
        let name = String::from_utf8_lossy(std::slice::from_raw_parts(base.add(name_at), end - name_at)).into_owned();
        Some((name, start))
    }
}

/// The calls that led here, innermost first (this module's own frames left out).
pub fn stack() -> String {
    let mut frames = [std::ptr::null_mut::<c_void>(); 40];
    let n = unsafe { RtlCaptureStackBackTrace(1, &mut frames, None) } as usize;
    frames[..n]
        .iter()
        .map(|&f| describe(f))
        .filter(|d| !d.starts_with("seamtype.dll"))
        .collect::<Vec<_>>()
        .join(" < ")
}

thread_local! {
    static HOOKS: RefCell<Vec<HHOOK>> = const { RefCell::new(Vec::new()) };
}

fn ime_message(message: u32, wparam: usize, lparam: isize) -> Option<String> {
    Some(match message {
        0x010D => "WM_IME_STARTCOMPOSITION".to_string(),
        0x010E => "WM_IME_ENDCOMPOSITION".to_string(),
        0x010F => format!("WM_IME_COMPOSITION flags {:#x}", lparam as u32),
        0x0281 => format!("WM_IME_SETCONTEXT active={wparam}"),
        0x0282 => format!("WM_IME_NOTIFY {wparam:#x}"),
        0x0283 => format!("WM_IME_CONTROL {wparam:#x}"),
        0x0285 => format!("WM_IME_SELECT {wparam}"),
        0x0286 => "WM_IME_CHAR".to_string(),
        0x0288 => format!("WM_IME_REQUEST {wparam:#x}"),
        0x0290 => "WM_IME_KEYDOWN".to_string(),
        0x0291 => "WM_IME_KEYUP".to_string(),
        0x0100 if wparam == 0xE5 => "WM_KEYDOWN processkey".to_string(),
        0x0101 if wparam == 0xE5 => "WM_KEYUP processkey".to_string(),
        0x0100 => "WM_KEYDOWN".to_string(),
        0x0102 => "WM_CHAR".to_string(),
        0x0007 => "WM_SETFOCUS".to_string(),
        0x0008 => "WM_KILLFOCUS".to_string(),
        _ => return None,
    })
}

unsafe extern "system" fn sent(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lparam.0 != 0 {
        let m = unsafe { &*(lparam.0 as *const CWPSTRUCT) };
        if let Some(what) = ime_message(m.message, m.wParam.0, m.lParam.0) {
            crate::log!("  sent {what}");
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn posted(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // wparam is PM_REMOVE when the message leaves the queue: log it once, then.
    if code >= 0 && wparam.0 == 1 && lparam.0 != 0 {
        let m = unsafe { &*(lparam.0 as *const MSG) };
        if let Some(what) = ime_message(m.message, m.wParam.0, m.lParam.0) {
            crate::log!("  posted {what}");
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Traces this thread's IME messages into the log.
pub fn install_hooks() {
    if !*crate::log::MESSAGES {
        return;
    }
    HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        if !hooks.is_empty() {
            return;
        }
        let thread = unsafe { GetCurrentThreadId() };
        unsafe {
            if let Ok(h) = SetWindowsHookExW(WH_CALLWNDPROC, Some(sent), None, thread) {
                hooks.push(h);
            }
            if let Ok(h) = SetWindowsHookExW(WH_GETMESSAGE, Some(posted), None, thread) {
                hooks.push(h);
            }
        }
    });
}

pub fn remove_hooks() {
    HOOKS.with(|hooks| {
        for h in hooks.borrow_mut().drain(..) {
            let _ = unsafe { UnhookWindowsHookEx(h) };
        }
    });
}
