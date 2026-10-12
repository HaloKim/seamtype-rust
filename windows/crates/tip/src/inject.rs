//! Backspaces Seamtype sends itself, for an IMM32 app that already has characters of a word that
//! must change, followed by a marker key: when the marker comes back through the text service,
//! the app has erased them and the word goes on. Both carry `MAGIC` as their extra information.

use std::sync::OnceLock;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_BACK, VK_F24,
};
use windows::Win32::UI::WindowsAndMessaging::GetMessageExtraInfo;

/// The extra information of keys Seamtype sends.
pub const MAGIC: usize = 0x5EA7_7E00;
/// The key that says the injected Backspaces have gone through.
pub const MARKER: u32 = VK_F24.0 as u32;
pub const BACKSPACE: u32 = VK_BACK.0 as u32;

/// Instead of sending real input (the harness): called with the number of Backspaces.
static INJECTOR: OnceLock<fn(usize)> = OnceLock::new();

pub fn set_injector(injector: fn(usize)) {
    let _ = INJECTOR.set(injector);
}

/// Whether the key event being handled is one Seamtype sent.
pub fn is_ours() -> bool {
    unsafe { GetMessageExtraInfo() }.0 as usize == MAGIC
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    }
}

/// Sends `count` Backspaces, then the marker.
pub fn erase_then_mark(count: usize) {
    if let Some(injector) = INJECTOR.get() {
        injector(count);
        return;
    }
    let mut inputs = Vec::with_capacity(count * 2 + 2);
    for _ in 0..count {
        inputs.push(key(VK_BACK, false));
        inputs.push(key(VK_BACK, true));
    }
    inputs.push(key(VK_F24, false));
    inputs.push(key(VK_F24, true));
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    crate::log!("sent {count} backspaces and the marker ({sent} of {} events)", inputs.len());
}
