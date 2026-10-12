//! Seamtype's button next to the language in the taskbar: the input menu of the macOS version.
//! Its icon says whether words are being decided (가A) or Seamtype is paused (A); a left click
//! pauses or resumes, a right click opens the menu (pause, the app's settings, Japanese, learned
//! words, the settings folder).

use std::cell::{Cell, RefCell};

use seamtype_core::AppPolicy;
use windows::Win32::Foundation::{E_INVALIDARG, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW, SelectObject, SetBkMode,
    SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE,
    DT_VCENTER, FW_BOLD, HBITMAP, HGDIOBJ, TRANSPARENT,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::TextServices::{
    ITfLangBarItemButton, ITfLangBarItemButton_Impl, ITfLangBarItemSink, ITfLangBarItem_Impl, ITfMenu, ITfSource,
    ITfSource_Impl, TfLBIClick, GUID_LBI_INPUTMODE, TF_LANGBARITEMINFO, TF_LBI_CLK_LEFT, TF_LBI_ICON,
    TF_LBI_STATUS, TF_LBI_STYLE_BTN_BUTTON, TF_LBI_STYLE_SHOWNINTRAY, TF_LBI_TEXT, TF_LBI_TOOLTIP,
    TF_LBMENUF_CHECKED, TF_LBMENUF_GRAYED, TF_LBMENUF_SEPARATOR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, GetCursorPos,
    MessageBoxW, RegisterClassW, TrackPopupMenuEx, HICON, ICONINFO, IDYES, MB_ICONINFORMATION,
    MB_ICONQUESTION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING,
    SW_SHOWNORMAL, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WNDCLASSW, WS_POPUP,
};
use windows_core::{implement, ComObject, IUnknown, Interface, Ref, Result, BOOL, BSTR, GUID, PCWSTR};

use crate::service::TextService;
use crate::{runtime, strings, CLSID_SEAMTYPE};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    TogglePause,
    ToggleConvert,
    Normal,
    Strict,
    ToggleEscape,
    UseDefault,
    ToggleJapanese,
    UndoLearned,
    ForgetAll,
    OpenFolder,
    About,
}

struct Item {
    text: String,
    checked: bool,
    enabled: bool,
    action: Option<Action>,
}

impl Item {
    fn new(text: impl Into<String>, action: Action) -> Item {
        Item { text: text.into(), checked: false, enabled: true, action: Some(action) }
    }

    fn separator() -> Item {
        Item { text: String::new(), checked: false, enabled: false, action: None }
    }

    fn header(text: String) -> Item {
        Item { text, checked: false, enabled: false, action: None }
    }

    fn checked(mut self, on: bool) -> Item {
        self.checked = on;
        self
    }

    fn enabled(mut self, on: bool) -> Item {
        self.enabled = on;
        self
    }

    fn is_separator(&self) -> bool {
        self.action.is_none() && self.text.is_empty()
    }
}

#[implement(ITfLangBarItemButton, ITfSource)]
pub struct LangBarButton {
    service: RefCell<Option<ComObject<TextService>>>,
    sink: RefCell<Option<ITfLangBarItemSink>>,
    /// The menu last built, so a choice maps back to its action.
    actions: RefCell<Vec<Option<Action>>>,
    shown_paused: Cell<Option<bool>>,
}

impl LangBarButton {
    pub fn new(service: ComObject<TextService>) -> LangBarButton {
        LangBarButton {
            service: RefCell::new(Some(service)),
            sink: RefCell::new(None),
            actions: RefCell::new(Vec::new()),
            shown_paused: Cell::new(None),
        }
    }

    /// The text service went away (deactivated): drop the reference that kept it alive.
    pub fn detach(&self) {
        self.service.borrow_mut().take();
        self.sink.borrow_mut().take();
    }

    fn service(&self) -> Option<ComObject<TextService>> {
        self.service.borrow().clone()
    }

    fn paused(&self) -> bool {
        self.service().map(|s| s.is_paused()).unwrap_or(false)
    }

    /// The pause state may have changed: refresh the icon if it did.
    pub fn changed(&self) {
        let paused = self.paused();
        if self.shown_paused.get() == Some(paused) {
            return;
        }
        self.shown_paused.set(Some(paused));
        if let Some(sink) = self.sink.borrow().as_ref() {
            let _ = unsafe { sink.OnUpdate(TF_LBI_ICON | TF_LBI_TEXT | TF_LBI_TOOLTIP | TF_LBI_STATUS) };
        }
    }

    fn items(&self) -> Vec<Item> {
        let s = strings::get();
        let Some(service) = self.service() else { return Vec::new() };
        let runtime = runtime::get();
        let mut items = vec![Item::new(s.pause, Action::TogglePause).checked(service.is_paused()), Item::separator()];
        let app = service.app();
        if !app.is_empty() {
            let policies = seamtype_core::AppPolicies::load_default();
            let policy = policies.policy(Some(&app));
            let listed = policies.overrides.contains_key(&app);
            let title = if listed { s.app_settings } else { s.default_settings };
            items.push(Item::header(strings::fill(title, &app)));
            items.push(Item::new(format!("\u{2003}{}", s.convert_words), Action::ToggleConvert).checked(policy.enabled));
            let strict = policy.conversion_threshold >= AppPolicy::STRICT_THRESHOLD;
            items.push(
                Item::new(format!("\u{2003}\u{2003}{}", s.conversion_normal), Action::Normal)
                    .checked(!strict)
                    .enabled(policy.enabled),
            );
            items.push(
                Item::new(format!("\u{2003}\u{2003}{}", s.conversion_strict), Action::Strict)
                    .checked(strict)
                    .enabled(policy.enabled),
            );
            items.push(
                Item::new(format!("\u{2003}\u{2003}{}", s.escape_to_app), Action::ToggleEscape)
                    .checked(policy.escape_to_app)
                    .enabled(policy.enabled),
            );
            if listed {
                items.push(Item::new(format!("\u{2003}{}", s.use_default), Action::UseDefault));
            }
            items.push(Item::separator());
        }
        items.push(Item::new(s.japanese, Action::ToggleJapanese).checked(runtime.settings.japanese));
        items.push(Item::separator());
        if let Some(keys) = runtime.engine.last_learned() {
            items.push(Item::new(strings::fill(s.undo_learned, &keys), Action::UndoLearned));
        }
        items.push(Item::new(s.forget_all, Action::ForgetAll));
        items.push(Item::new(s.open_folder, Action::OpenFolder));
        items.push(Item::new(s.about, Action::About));
        items
    }

    fn perform(&self, action: Action) {
        let Some(service) = self.service() else { return };
        let app = service.app();
        match action {
            Action::TogglePause => {
                service.toggle_pause();
                return;
            }
            Action::ToggleConvert => runtime::get().update_policy(&app, |p| p.enabled = !p.enabled),
            Action::Normal => runtime::get().update_policy(&app, |p| p.conversion_threshold = AppPolicy::STANDARD.conversion_threshold),
            Action::Strict => runtime::get().update_policy(&app, |p| p.conversion_threshold = AppPolicy::STRICT_THRESHOLD),
            Action::ToggleEscape => runtime::get().update_policy(&app, |p| p.escape_to_app = !p.escape_to_app),
            Action::UseDefault => runtime::get().remove_policy(&app),
            Action::ToggleJapanese => runtime::get().update(|s| s.japanese = !s.japanese),
            Action::UndoLearned => runtime::get().undo_last_learned(),
            Action::ForgetAll => {
                let s = strings::get();
                let text = wide(&format!("{}\n\n{}", s.forget_all_question, s.forget_all_detail));
                let title = wide("Seamtype");
                let answer = unsafe {
                    MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND | MB_TOPMOST)
                };
                if answer == IDYES {
                    runtime::get().engine.forget_all();
                }
            }
            Action::OpenFolder => {
                if let Some(dir) = seamtype_core::support::directory() {
                    let _ = std::fs::create_dir_all(&dir);
                    let path = wide(&dir.to_string_lossy());
                    let verb = wide("open");
                    unsafe {
                        ShellExecuteW(None, PCWSTR(verb.as_ptr()), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
                    }
                }
            }
            Action::About => {
                let s = strings::get();
                let text = wide(&strings::fill(s.about_text, env!("CARGO_PKG_VERSION")));
                let title = wide(s.about);
                unsafe {
                    MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND | MB_TOPMOST);
                }
            }
        }
        service.settings_changed();
    }

    /// Shows the menu at `point` and performs the choice.
    fn popup(&self, point: POINT) {
        let items = self.items();
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            for (i, item) in items.iter().enumerate() {
                if item.is_separator() {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                    continue;
                }
                let mut flags = MF_STRING;
                if item.checked {
                    flags |= MF_CHECKED;
                }
                if !item.enabled || item.action.is_none() {
                    flags |= MF_GRAYED;
                }
                let text = wide(&item.text);
                let _ = AppendMenuW(menu, flags, i + 1, PCWSTR(text.as_ptr()));
            }
            let owner = menu_owner();
            let chosen = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON).0, point.x, point.y, owner, None);
            let _ = DestroyMenu(menu);
            let chosen = chosen.0 as usize;
            if chosen >= 1 {
                if let Some(action) = items.get(chosen - 1).and_then(|i| i.action) {
                    self.perform(action);
                }
            }
        }
    }
}

unsafe extern "system" fn owner_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// A hidden window of this thread to own the menu.
fn menu_owner() -> HWND {
    thread_local! {
        static OWNER: Cell<isize> = const { Cell::new(0) };
    }
    OWNER.with(|owner| {
        if owner.get() != 0 {
            return HWND(owner.get() as *mut _);
        }
        unsafe {
            let class = wide("SeamtypeMenuOwner");
            let instance = crate::module();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(owner_proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0x80), // WS_EX_TOOLWINDOW
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
            owner.set(hwnd.0 as isize);
            hwnd
        }
    })
}

/// Whether the taskbar is light (dark glyphs read on it).
fn light_taskbar() -> bool {
    let mut value = 0u32;
    let mut size = 4u32;
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = wide("SystemUsesLightTheme");
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok() && value != 0
}

/// A square icon with `text` drawn in the taskbar's glyph color, alpha from the glyph's coverage.
fn text_icon(text: &str) -> Result<HICON> {
    const SIZE: i32 = 32;
    unsafe {
        let dc = CreateCompatibleDC(None);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE,
                biHeight: -SIZE,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color: HBITMAP = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, HGDIOBJ(color.0));
        let face = wide("Segoe UI");
        let height = if text.chars().count() > 1 { -19 } else { -26 };
        let font = CreateFontW(height, 0, 0, 0, FW_BOLD.0 as i32, 0, 0, 0, Default::default(), Default::default(), Default::default(), CLEARTYPE_QUALITY, Default::default(), PCWSTR(face.as_ptr()));
        let old_font = SelectObject(dc, HGDIOBJ(font.0));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
        let mut rect = RECT { left: 0, top: 0, right: SIZE, bottom: SIZE };
        let mut glyphs: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(dc, &mut glyphs, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        SelectObject(dc, old);
        let _ = DeleteDC(dc);

        // White glyphs on black: coverage becomes alpha, the color the taskbar's.
        let (r, g, b) = if light_taskbar() { (0u32, 0u32, 0u32) } else { (255u32, 255u32, 255u32) };
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (SIZE * SIZE) as usize);
        for p in pixels.iter_mut() {
            let coverage = ((*p >> 16 & 0xFF) + (*p >> 8 & 0xFF) + (*p & 0xFF)) / 3;
            *p = coverage << 24 | r << 16 | g << 8 | b;
        }
        let mask = windows::Win32::Graphics::Gdi::CreateBitmap(SIZE, SIZE, 1, 1, None);
        let icon_info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&icon_info);
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteObject(HGDIOBJ(color.0));
        icon
    }
}

impl ITfLangBarItem_Impl for LangBarButton_Impl {
    fn GetInfo(&self, info: *mut TF_LANGBARITEMINFO) -> Result<()> {
        if info.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let mut description = [0u16; 32];
        for (i, c) in "Seamtype".encode_utf16().enumerate() {
            description[i] = c;
        }
        unsafe {
            *info = TF_LANGBARITEMINFO {
                clsidService: CLSID_SEAMTYPE,
                guidItem: GUID_LBI_INPUTMODE,
                dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY,
                ulSort: 0,
                szDescription: description,
            };
        }
        Ok(())
    }

    fn GetStatus(&self) -> Result<u32> {
        Ok(0)
    }

    fn Show(&self, _show: BOOL) -> Result<()> {
        Ok(())
    }

    fn GetTooltipString(&self) -> Result<BSTR> {
        let s = strings::get();
        Ok(BSTR::from(if self.paused() { s.tooltip_paused } else { s.tooltip_on }))
    }
}

impl ITfLangBarItemButton_Impl for LangBarButton_Impl {
    fn OnClick(&self, click: TfLBIClick, point: &POINT, _area: *const RECT) -> Result<()> {
        crate::guard(|| {
            if click == TF_LBI_CLK_LEFT {
                if let Some(service) = self.service() {
                    service.toggle_pause();
                }
            } else {
                let mut at = *point;
                if at.x == 0 && at.y == 0 {
                    let _ = unsafe { GetCursorPos(&mut at) };
                }
                self.popup(at);
            }
            Ok(())
        })
    }

    fn InitMenu(&self, menu: Ref<ITfMenu>) -> Result<()> {
        crate::guard(|| {
            let menu = menu.ok()?;
            let items = self.items();
            let mut actions = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let mut flags = 0;
                if item.is_separator() {
                    flags |= TF_LBMENUF_SEPARATOR;
                }
                if item.checked {
                    flags |= TF_LBMENUF_CHECKED;
                }
                if !item.is_separator() && (!item.enabled || item.action.is_none()) {
                    flags |= TF_LBMENUF_GRAYED;
                }
                let text: Vec<u16> = item.text.encode_utf16().collect();
                unsafe {
                    let _ = menu.AddMenuItem(i as u32, flags, HBITMAP::default(), HBITMAP::default(), &text, std::ptr::null_mut());
                }
                actions.push(item.action);
            }
            *self.actions.borrow_mut() = actions;
            Ok(())
        })
    }

    fn OnMenuSelect(&self, id: u32) -> Result<()> {
        crate::guard(|| {
            let action = self.actions.borrow().get(id as usize).copied().flatten();
            if let Some(action) = action {
                self.perform(action);
            }
            Ok(())
        })
    }

    fn GetIcon(&self) -> Result<HICON> {
        crate::guard(|| text_icon(if self.paused() { "A" } else { "가A" }))
    }

    fn GetText(&self) -> Result<BSTR> {
        Ok(BSTR::from("Seamtype"))
    }
}

impl ITfSource_Impl for LangBarButton_Impl {
    fn AdviseSink(&self, riid: *const GUID, sink: Ref<IUnknown>) -> Result<u32> {
        crate::guard(|| {
            if riid.is_null() || unsafe { *riid } != ITfLangBarItemSink::IID {
                return Err(windows::Win32::System::Ole::CONNECT_E_CANNOTCONNECT.into());
            }
            let sink: ITfLangBarItemSink = sink.ok()?.cast()?;
            *self.sink.borrow_mut() = Some(sink);
            Ok(1)
        })
    }

    fn UnadviseSink(&self, cookie: u32) -> Result<()> {
        if cookie == 1 {
            self.sink.borrow_mut().take();
        }
        Ok(())
    }
}

