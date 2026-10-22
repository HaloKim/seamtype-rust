//! The word being typed in an IMM32 app, shown in a small window at the caret (see client.rs):
//! the app gets nothing of the word until it ends, so nothing it has ever has to be erased.
//!
//! One window per UI thread, never activated, so the app keeps focus and its caret.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetDC,
    GetMonitorInfoW, GetTextExtentPoint32W, LineTo, MonitorFromPoint, MoveToEx, ReleaseDC, SelectObject, SetBkMode,
    SetTextColor, TextOutW, CLEARTYPE_QUALITY, FW_NORMAL, HFONT, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetGUIThreadInfo, RegisterClassW, SetWindowPos, ShowWindow,
    GUITHREADINFO, HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
    WM_MOUSEACTIVATE, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows_core::PCWSTR;

/// Tests (the harness) see what would be shown without a window on the screen.
static HEADLESS: AtomicBool = AtomicBool::new(false);
static SHOWN: Mutex<Option<String>> = Mutex::new(None);

pub fn set_headless(on: bool) {
    HEADLESS.store(on, Ordering::SeqCst);
}

/// What the window shows now, if anything.
pub fn shown() -> Option<String> {
    SHOWN.lock().ok().and_then(|s| s.clone())
}

struct Popup {
    hwnd: isize,
    text: Vec<u16>,
    /// Font height in pixels, from the caret's height.
    height: i32,
}

thread_local! {
    static POPUP: RefCell<Popup> = const { RefCell::new(Popup { hwnd: 0, text: Vec::new(), height: 0 }) };
}

const PAD_X: i32 = 6;
const PAD_Y: i32 = 3;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn font(height: i32) -> HFONT {
    let face = wide("Malgun Gothic");
    unsafe {
        CreateFontW(-height, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, Default::default(), Default::default(),
            Default::default(), CLEARTYPE_QUALITY, 0, PCWSTR(face.as_ptr()))
    }
}

/// Whether apps are in light mode (the window follows them).
fn light() -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = wide("AppsUseLightTheme");
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_DWORD, None,
            Some(&mut value as *mut u32 as *mut _), Some(&mut size))
    };
    status.is_err() || value != 0
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

unsafe extern "system" fn popup_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            let (text, height) = POPUP.with(|p| {
                let p = p.borrow();
                (p.text.clone(), p.height)
            });
            unsafe {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut client = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
                let (back, fore, edge) = if light() {
                    (rgb(255, 255, 255), rgb(24, 24, 24), rgb(0, 120, 212))
                } else {
                    (rgb(43, 43, 43), rgb(240, 240, 240), rgb(96, 205, 255))
                };
                let brush = CreateSolidBrush(edge);
                FillRect(dc, &client, brush);
                let _ = DeleteObject(HGDIOBJ(brush.0));
                let inner = RECT { left: 1, top: 1, right: client.right - 1, bottom: client.bottom - 1 };
                let brush = CreateSolidBrush(back);
                FillRect(dc, &inner, brush);
                let _ = DeleteObject(HGDIOBJ(brush.0));
                let f = font(height);
                let old = SelectObject(dc, HGDIOBJ(f.0));
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, fore);
                let _ = TextOutW(dc, PAD_X, PAD_Y, &text);
                // Underlined, as composed text is.
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(dc, &text, &mut size);
                let pen = CreatePen(PS_SOLID, 1, fore);
                let old_pen = SelectObject(dc, HGDIOBJ(pen.0));
                let y = PAD_Y + size.cy - 1;
                let _ = MoveToEx(dc, PAD_X, y, None);
                let _ = LineTo(dc, PAD_X + size.cx, y);
                SelectObject(dc, old_pen);
                let _ = DeleteObject(HGDIOBJ(pen.0));
                SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(f.0));
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn window() -> HWND {
    POPUP.with(|p| {
        let mut p = p.borrow_mut();
        if p.hwnd != 0 {
            return HWND(p.hwnd as *mut _);
        }
        unsafe {
            let class = wide("SeamtypeWord");
            let instance = crate::module();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(popup_proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap_or_default();
            p.hwnd = hwnd.0 as isize;
            hwnd
        }
    })
}

/// The caret of this thread on the screen, when the app keeps a system caret.
fn system_caret() -> Option<RECT> {
    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    unsafe {
        let thread = windows::Win32::System::Threading::GetCurrentThreadId();
        GetGUIThreadInfo(thread, &mut info).ok()?;
        if info.hwndCaret.0.is_null() {
            return None;
        }
        let mut a = POINT { x: info.rcCaret.left, y: info.rcCaret.top };
        let mut b = POINT { x: info.rcCaret.right, y: info.rcCaret.bottom };
        let _ = ClientToScreen(info.hwndCaret, &mut a);
        let _ = ClientToScreen(info.hwndCaret, &mut b);
        Some(RECT { left: a.x, top: a.y, right: b.x, bottom: b.y })
    }
}

/// Shows `text` under the caret (`caret`, on the screen, as the document reports it).
pub fn show(text: &str, caret: Option<RECT>) {
    if let Ok(mut shown) = SHOWN.lock() {
        *shown = Some(text.to_string());
    }
    if HEADLESS.load(Ordering::SeqCst) {
        return;
    }
    let caret = caret.filter(|r| r.bottom > r.top).or_else(system_caret);
    let caret_height = caret.map(|r| r.bottom - r.top).unwrap_or(20);
    let height = caret_height.clamp(14, 40) * 4 / 5;
    let hwnd = window();
    if hwnd.0.is_null() {
        return;
    }
    let text16: Vec<u16> = text.encode_utf16().collect();
    let size = unsafe {
        let dc = GetDC(Some(hwnd));
        let f = font(height);
        let old = SelectObject(dc, HGDIOBJ(f.0));
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &text16, &mut size);
        SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(f.0));
        ReleaseDC(Some(hwnd), dc);
        size
    };
    POPUP.with(|p| {
        let mut p = p.borrow_mut();
        p.text = text16;
        p.height = height;
    });
    let (w, h) = (size.cx + 2 * PAD_X, size.cy + 2 * PAD_Y);
    let anchor = caret.unwrap_or_else(|| {
        let mut point = POINT::default();
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) };
        RECT { left: point.x, top: point.y, right: point.x, bottom: point.y + 20 }
    });
    let (mut x, mut y) = (anchor.left, anchor.bottom + 2);
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: anchor.left, y: anchor.top }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let work = info.rcWork;
            if y + h > work.bottom {
                y = anchor.top - h - 2;
            }
            x = x.min(work.right - w).max(work.left);
        }
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

pub fn hide() {
    if let Ok(mut shown) = SHOWN.lock() {
        *shown = None;
    }
    POPUP.with(|p| {
        let p = p.borrow();
        if p.hwnd != 0 {
            unsafe {
                let _ = ShowWindow(HWND(p.hwnd as *mut _), SW_HIDE);
            }
        }
    });
}

/// The text service is going away on this thread.
pub fn destroy() {
    hide();
    POPUP.with(|p| {
        let mut p = p.borrow_mut();
        if p.hwnd != 0 {
            unsafe {
                let _ = DestroyWindow(HWND(p.hwnd as *mut _));
            }
            p.hwnd = 0;
        }
    });
}
