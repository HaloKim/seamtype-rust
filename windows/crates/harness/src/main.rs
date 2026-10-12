//! End-to-end checks of the text service through Windows' own Text Services Framework.
//!
//! The harness plays the app: it makes TSF documents over a text store of its own (store.rs),
//! activates Seamtype's text service on its thread the way TSF does for an input method, and
//! sends key events through TSF's keystroke manager. Keys the text service passes on are typed
//! by the "app", as a real text field would. Nothing is registered and no real keyboard input
//! or window focus is involved, so it runs on a machine in use.
//!
//!   seamtype-harness               run every scenario, print each result, exit 1 on a failure
//!   seamtype-harness --registered  the same through the installed input method: TSF loads it
//!                                  from the registry for this process and routes keys and focus
//!   seamtype-harness <script>...   type each script into a new field and show what it left

mod store;

use std::process::ExitCode;

use seamtype::{TextService, CLSID_SEAMTYPE};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use store::Store;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, GUID_TFCAT_TIP_KEYBOARD, ITextStoreACP, ITfClientId, ITfContext,
    ITfContextOwnerCompositionServices,
    ITfDocumentMgr, ITfInputProcessorProfileMgr, ITfKeyEventSink, ITfKeystrokeMgr, ITfThreadMgr, ITfThreadMgrEventSink,
    ITfThreadMgrEx, TF_INPUTPROCESSORPROFILE, TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE, TF_IPPMF_FORPROCESS,
    TF_PROFILETYPE_INPUTPROCESSOR, TF_TMAE_NOACTIVATETIP,
};
use windows_core::{ComObject, IUnknown, Interface, Result};

#[derive(Clone)]
struct Document {
    manager: ITfDocumentMgr,
    context: ITfContext,
    store: ComObject<Store>,
}

/// How keys and focus reach the text service.
enum Route {
    /// Its sinks are called directly, as TSF calls them (TSF only routes keys and focus to an
    /// input method it activated itself, from the registry).
    Direct { keys: ITfKeyEventSink, focus: ITfThreadMgrEventSink },
    /// The installed input method, activated by TSF for this process: TSF routes everything.
    Tsf { profiles: ITfInputProcessorProfileMgr, previous: Option<TF_INPUTPROCESSORPROFILE> },
}

struct Harness {
    thread_mgr: ITfThreadMgr,
    app_client: u32,
    keystrokes: ITfKeystrokeMgr,
    route: Route,
    /// Every field made so far stays alive, as an app's text fields do.
    documents: std::cell::RefCell<Vec<Document>>,
}

/// What a script character presses: the key, Shift, and what the app types if it gets the key.
enum Typed {
    Text(String),
    Newline,
    Backspace,
    Nothing,
}

fn press_for(ch: char) -> (VIRTUAL_KEY, bool, Typed) {
    let text = |c: char| Typed::Text(c.to_string());
    match ch {
        'a'..='z' => (VIRTUAL_KEY(0x41 + (ch as u16 - 'a' as u16)), false, text(ch)),
        'A'..='Z' => (VIRTUAL_KEY(0x41 + (ch as u16 - 'A' as u16)), true, text(ch)),
        '0'..='9' => (VIRTUAL_KEY(0x30 + (ch as u16 - '0' as u16)), false, text(ch)),
        ' ' => (VK_SPACE, false, text(ch)),
        '⏎' => (VK_RETURN, false, Typed::Newline),
        '⎋' => (VK_ESCAPE, false, Typed::Nothing),
        '⇧' => (VK_ESCAPE, true, Typed::Nothing),
        '⌫' => (VK_BACK, false, Typed::Backspace),
        '←' => (VK_LEFT, false, Typed::Nothing),
        '→' => (VK_RIGHT, false, Typed::Nothing),
        '⇄' => (VK_HANGUL, false, Typed::Nothing),
        _ => {
            let shifted = ")!@#$%^&*(";
            if let Some(i) = shifted.find(ch) {
                return (VIRTUAL_KEY(0x30 + i as u16), true, text(ch));
            }
            let (vk, shift) = match ch {
                ',' => (VK_OEM_COMMA, false),
                '<' => (VK_OEM_COMMA, true),
                '.' => (VK_OEM_PERIOD, false),
                '>' => (VK_OEM_PERIOD, true),
                '/' => (VK_OEM_2, false),
                '?' => (VK_OEM_2, true),
                ';' => (VK_OEM_1, false),
                ':' => (VK_OEM_1, true),
                '\'' => (VK_OEM_7, false),
                '"' => (VK_OEM_7, true),
                '[' => (VK_OEM_4, false),
                '{' => (VK_OEM_4, true),
                ']' => (VK_OEM_6, false),
                '}' => (VK_OEM_6, true),
                '\\' => (VK_OEM_5, false),
                '|' => (VK_OEM_5, true),
                '-' => (VK_OEM_MINUS, false),
                '_' => (VK_OEM_MINUS, true),
                '=' => (VK_OEM_PLUS, false),
                '+' => (VK_OEM_PLUS, true),
                '`' => (VK_OEM_3, false),
                '~' => (VK_OEM_3, true),
                _ => panic!("no key for {ch:?}"),
            };
            (vk, shift, text(ch))
        }
    }
}

impl Harness {
    fn new(registered: bool) -> Result<Harness> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let step = |what: &'static str| move |e: windows_core::Error| {
                println!("  setup step failed: {what}: {e}");
                e
            };
            let thread_mgr: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).map_err(step("create thread manager"))?;
            let keystrokes: ITfKeystrokeMgr = thread_mgr.cast()?;
            if registered {
                let app_client = thread_mgr.Activate().map_err(step("activate thread manager"))?;
                let profiles: ITfInputProcessorProfileMgr =
                    CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
                let mut previous = TF_INPUTPROCESSORPROFILE::default();
                let previous = profiles.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut previous).ok().map(|_| previous);
                profiles
                    .ActivateProfile(
                        TF_PROFILETYPE_INPUTPROCESSOR,
                        seamtype::LANGID_KOREAN,
                        &CLSID_SEAMTYPE,
                        &seamtype::GUID_PROFILE,
                        HKL::default(),
                        TF_IPPMF_FORPROCESS | TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE,
                    )
                    .map_err(step("activate Seamtype's profile (is it installed?)"))?;
                let route = Route::Tsf { profiles, previous };
                let harness = Harness { thread_mgr, app_client, keystrokes, route, documents: Default::default() };
                harness.focus_window();
                return Ok(harness);
            }
            // No input method of the system's: only Seamtype's text service, activated below.
            let mut app_client = 0u32;
            thread_mgr
                .cast::<ITfThreadMgrEx>()?
                .ActivateEx(&mut app_client, TF_TMAE_NOACTIVATETIP)
                .map_err(step("activate thread manager"))?;
            let tip_client =
                thread_mgr.cast::<ITfClientId>()?.GetClientId(&CLSID_SEAMTYPE).map_err(step("client id"))?;
            let tip: windows::Win32::UI::TextServices::ITfTextInputProcessorEx = TextService::new().into();
            tip.ActivateEx(&thread_mgr, tip_client, 0).map_err(step("activate text service"))?;
            let route = Route::Direct { keys: tip.cast()?, focus: tip.cast()? };
            Ok(Harness { thread_mgr, app_client, keystrokes, route, documents: Default::default() })
        }
    }

    /// Gives the thread a focused window of its own, off screen and never activated, so TSF
    /// treats it as a thread that types (the user's foreground window is left alone).
    fn focus_window(&self) {
        use windows::Win32::UI::WindowsAndMessaging::*;
        unsafe {
            let class: Vec<u16> = "SeamtypeHarness\0".encode_utf16().collect();
            let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: windows_core::PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                windows_core::PCWSTR(class.as_ptr()),
                windows_core::PCWSTR::null(),
                WS_POPUP,
                -32000,
                -32000,
                10,
                10,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap_or_default();
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let focused = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(hwnd));
            println!("  focus window: {:?} (set focus: {})", hwnd, focused.is_ok());
            *FOCUS_WINDOW.lock().unwrap() = hwnd.0 as isize;
        }
    }

    /// Puts back the input method this process had (registered runs only).
    fn restore(&self) {
        if let Route::Tsf { profiles, previous: Some(p) } = &self.route {
            unsafe {
                let _ = profiles.ActivateProfile(p.dwProfileType, p.langid, &p.clsid, &p.guidProfile, p.hkl, TF_IPPMF_FORPROCESS);
            }
        }
    }

    /// A new, focused text field.
    fn document(&self) -> Result<Document> {
        self.document_with(false)
    }

    /// A new, focused text field; `transitory` makes it an IMM32 app's, as Windows bridges them.
    fn document_with(&self, transitory: bool) -> Result<Document> {
        unsafe {
            let manager = self.thread_mgr.CreateDocumentMgr()?;
            let store = ComObject::new(Store::new());
            store.transitory.set(transitory);
            let unknown: IUnknown = store.to_interface::<ITextStoreACP>().cast()?;
            let mut context = None;
            let mut cookie = 0u32;
            manager.CreateContext(self.app_client, 0, &unknown, &mut context, &mut cookie)?;
            let context = context.expect("context");
            manager.Push(&context)?;
            let window = *FOCUS_WINDOW.lock().unwrap();
            if window != 0 {
                let _ = self.thread_mgr.AssociateFocus(windows::Win32::Foundation::HWND(window as *mut _), &manager);
            }
            let previous = self.documents.borrow().last().map(|d| d.manager.clone());
            self.thread_mgr.SetFocus(&manager)?;
            if let Route::Direct { focus, .. } = &self.route {
                focus.OnSetFocus(&manager, previous.as_ref())?;
            }
            let document = Document { manager, context, store };
            self.documents.borrow_mut().push(document.clone());
            Ok(document)
        }
    }

    fn set_shift(&self, shift: bool) {
        let mut state = [0u8; 256];
        unsafe {
            let _ = GetKeyboardState(&mut state);
        }
        let v = if shift { 0x80 } else { 0 };
        state[VK_SHIFT.0 as usize] = v;
        state[VK_LSHIFT.0 as usize] = v;
        state[VK_CAPITAL.0 as usize] = 0;
        state[VK_CONTROL.0 as usize] = 0;
        state[VK_MENU.0 as usize] = 0;
        unsafe {
            let _ = SetKeyboardState(&state);
        }
    }

    /// Presses one key as an app's message loop would hand it to TSF; returns whether the text
    /// service ate it.
    fn press(&self, document: &Document, ch: char) -> bool {
        self.press_with(document, ch, 1, false)
    }

    /// `deliveries` times the same key-down (as some apps hand it over), then its key-up;
    /// `repeat` flags the key-down as auto-repeat.
    fn press_with(&self, document: &Document, ch: char, deliveries: usize, repeat: bool) -> bool {
        let (vk, shift, typed) = press_for(ch);
        self.set_shift(shift);
        let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) };
        let wparam = WPARAM(vk.0 as usize);
        let down = LPARAM((1 | (scan << 16) | if repeat { 1 << 30 } else { 0 }) as isize);
        let up = LPARAM((1 | (scan << 16) | (0xC000_0000u32)) as isize);
        let context = &document.context;
        let deliver = || unsafe {
            match &self.route {
                Route::Direct { keys, .. } if vk == VK_HANGUL => {
                    keys.OnPreservedKey(context, &seamtype::GUID_PRESERVED_PAUSE).map(|b| b.as_bool()).unwrap_or(false)
                }
                Route::Direct { keys, .. } => {
                    let test = keys.OnTestKeyDown(context, wparam, down).map(|b| b.as_bool()).unwrap_or(false);
                    test && keys.OnKeyDown(context, wparam, down).map(|b| b.as_bool()).unwrap_or(false)
                }
                Route::Tsf { .. } => {
                    let test = self.keystrokes.TestKeyDown(wparam, down).map(|b| b.as_bool()).unwrap_or(false);
                    test && self.keystrokes.KeyDown(wparam, down).map(|b| b.as_bool()).unwrap_or(false)
                }
            }
        };
        let mut eaten = deliver();
        for _ in 1..deliveries {
            eaten = deliver() || eaten;
        }
        if !eaten {
            match typed {
                Typed::Text(s) => document.store.app_insert(&s),
                Typed::Newline => document.store.app_insert("\n"),
                Typed::Backspace => document.store.app_backspace(),
                Typed::Nothing => {}
            }
        }
        unsafe {
            match &self.route {
                Route::Direct { keys, .. } => {
                    if keys.OnTestKeyUp(context, wparam, up).map(|b| b.as_bool()).unwrap_or(false) {
                        let _ = keys.OnKeyUp(context, wparam, up);
                    }
                }
                Route::Tsf { .. } => {
                    if self.keystrokes.TestKeyUp(wparam, up).map(|b| b.as_bool()).unwrap_or(false) {
                        let _ = self.keystrokes.KeyUp(wparam, up);
                    }
                }
            }
        }
        self.set_shift(false);
        // Backspaces the text service sent (to an IMM32 app): the app erases, then the marker.
        let injected: Vec<usize> = std::mem::take(&mut *INJECTED.lock().unwrap());
        for count in injected {
            for _ in 0..count {
                document.store.app_backspace();
            }
            self.marker(document);
        }
        eaten
    }

    /// Seamtype's marker key coming back.
    fn marker(&self, document: &Document) {
        let wparam = WPARAM(VK_F24.0 as usize);
        let down = LPARAM(1);
        let up = LPARAM((1u32 | 0xC000_0000) as isize);
        if let Route::Direct { keys, .. } = &self.route {
            unsafe {
                if keys.OnTestKeyDown(&document.context, wparam, down).map(|b| b.as_bool()).unwrap_or(false) {
                    let _ = keys.OnKeyDown(&document.context, wparam, down);
                }
                if keys.OnTestKeyUp(&document.context, wparam, up).map(|b| b.as_bool()).unwrap_or(false) {
                    let _ = keys.OnKeyUp(&document.context, wparam, up);
                }
            }
        }
    }

    fn type_script(&self, document: &Document, script: &str) {
        for ch in script.chars() {
            self.press(document, ch);
        }
    }
}

static FOCUS_WINDOW: std::sync::Mutex<isize> = std::sync::Mutex::new(0);

/// Backspaces the text service asked to send, instead of real input.
static INJECTED: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

fn inject(count: usize) {
    INJECTED.lock().unwrap().push(count);
}

unsafe extern "system" fn window_proc(
    hwnd: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    unsafe { windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, message, wparam, lparam) }
}

struct Report {
    passed: usize,
    failed: usize,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: String) {
        if ok {
            self.passed += 1;
            println!("ok    {name}");
        } else {
            self.failed += 1;
            println!("FAIL  {name}: {detail}");
        }
    }
}

fn main() -> ExitCode {
    // Settings, rules and learned words of the run stay in a folder of its own.
    let dir = std::env::temp_dir().join(format!("seamtype-harness-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    unsafe { std::env::set_var("SEAMTYPE_SUPPORT_DIR", &dir) };

    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let registered = args.first().map(|a| a == "--registered").unwrap_or(false);
    if registered {
        args.remove(0);
    }
    seamtype::set_injector(inject);
    let harness = match Harness::new(registered) {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  setting up TSF: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut report = Report { passed: 0, failed: 0 };

    // `seamtype-harness <script>...`: type each script into a new field and show what it left.
    let scripts = args;
    if !scripts.is_empty() {
        for script in scripts {
            if let Ok(document) = harness.document() {
                harness.type_script(&document, &script);
                println!("{script:?} → {:?} (compositions {})", document.store.text(), document.store.compositions.get());
            }
        }
        harness.restore();
        return ExitCode::SUCCESS;
    }

    // Typed scripts and the text they leave (the macOS end-to-end expectations).
    let scripts: &[(&str, &str)] = &[
        ("dkssudgktpdy hello ", "안녕하세요 hello "),
        ("dlrjs githubdp commitgkf rjdi.", "이건 github에 commit할 거야."),
        ("hellodnjfem githubtjqj ", "hello월드 github서버 "),
        ("xdp ndl Xfmf Xeh ", "x에 n이 X를 X도 "),
        ("Hello, World! ", "Hello, World! "),
        ("dkssudgktpdy. qksrkqtmqslek!", "안녕하세요. 반갑습니다!"),
        ("dhsmf 3tl", "오늘 3시"),
        ("hello ⎋", "ㅗ디ㅣㅐ "),
        ("hello ⎋⎋", "hello "),
        ("dkssud⎋ ", "dkssud "),
        // 거스 corrected to rjtm twice is learned: the third rjtm stays rjtm without an Esc.
        ("rjtm ⎋x rjtm ⎋x rjtm ", "rjtm x rjtm x rjtm "),
        ("gksrnr⌫", "한구"),
        ("gksrnr⌫⌫⌫⌫", ""),
        ("dkssudgktpdy⌫b ", "안녕하세유 "),
        ("dkssud⏎", "안녕"),
        ("dkssud⏎⏎", "안녕\n"),
        ("dkssud⇄dkssud ", "안녕dkssud "),
        ("dkssud⇄jk ⇄dkssud ", "안녕jk 안녕 "),
        ("githubdp ⎋", "githubdp "),
        ("don't ", "don't "),
    ];
    for (script, expected) in scripts {
        let document = match harness.document() {
            Ok(d) => d,
            Err(e) => {
                report.check(script, false, format!("document: {e}"));
                continue;
            }
        };
        harness.type_script(&document, script);
        let text = document.store.text();
        report.check(script, text == *expected, format!("got {text:?}, expected {expected:?}"));
    }

    // An IMM32 app (Windows' bridge, Korean rules: one interim character composed, the rest the
    // app's as typed): the same words come out, nothing twice.
    let imm_scripts: &[(&str, &str)] = &[
        ("dkssudgktpdy hello ", "안녕하세요 hello "),
        ("dlrjs githubdp commitgkf rjdi.", "이건 github에 commit할 거야."),
        ("hellodnjfem githubtjqj ", "hello월드 github서버 "),
        ("gksrmf⎋ ", "gksrmf "),
        ("gksrnr⌫", "한구"),
        ("gksrnr⌫⌫⌫⌫", ""),
        ("dkssudgktpdy⌫b ", "안녕하세유 "),
        ("dkssud⏎", "안녕"),
        ("dkssud⏎⏎", "안녕\n"),
        ("Hello, World! ", "Hello, World! "),
    ];
    for (script, expected) in imm_scripts {
        let Ok(document) = harness.document_with(true) else { continue };
        let mut longest = 0usize;
        for ch in script.chars() {
            harness.press(&document, ch);
            longest = longest.max(document.store.compositions.get().max(0) as usize);
        }
        let text = document.store.text();
        report.check(&format!("imm: {script}"), text == *expected && longest <= 1, format!("got {text:?}, expected {expected:?}"));
    }

    // While a word is composed the field holds a composition (underlined), ended by the commit.
    if let Ok(document) = harness.document() {
        harness.type_script(&document, "dkssud");
        let composing = document.store.compositions.get();
        let shown = document.store.text();
        harness.type_script(&document, " ");
        let after = document.store.compositions.get();
        report.check(
            "composition while typing",
            composing == 1 && shown == "안녕" && after == 0 && document.store.text() == "안녕 ",
            format!("compositions {composing} → {after}, text {shown:?} → {:?}", document.store.text()),
        );
    }

    // An app that hands every key press over twice (Qt apps through IMM32): each press counts once.
    if let Ok(document) = harness.document() {
        for ch in "dkssudgktpdy ".chars() {
            harness.press_with(&document, ch, 2, false);
        }
        report.check(
            "key presses delivered twice",
            document.store.text() == "안녕하세요 ",
            format!("got {:?}", document.store.text()),
        );
    }

    // An app that ends every composition as soon as it shows (Webex, a Qt app, did): what it
    // showed stays, once; nothing is typed twice.
    if let Ok(document) = harness.document() {
        let services: Option<ITfContextOwnerCompositionServices> = document.context.cast().ok();
        for ch in "dkssud".chars() {
            harness.press(&document, ch);
            if let Some(services) = &services {
                let _ = unsafe { services.TerminateComposition(None) };
            }
        }
        let text = document.store.text();
        report.check(
            "app ends each composition",
            text.chars().count() == 6 && document.store.compositions.get() == 0,
            format!("got {text:?} ({} compositions)", document.store.compositions.get()),
        );
    }

    // Auto-repeat is a new press every time, even without a key-up in between.
    if let Ok(document) = harness.document() {
        harness.press_with(&document, 'z', 1, false);
        harness.press_with(&document, 'z', 1, true);
        harness.press_with(&document, 'z', 1, true);
        harness.press(&document, ' ');
        report.check("auto-repeat", document.store.text() == "ㅋㅋㅋ ", format!("got {:?}", document.store.text()));
    }

    // A click elsewhere closes the flip window: Esc goes to the app.
    if let Ok(document) = harness.document() {
        harness.type_script(&document, "dkssud ");
        document.store.app_move_caret(0);
        let eaten = harness.press(&document, '⎋');
        report.check(
            "esc after the caret moved",
            !eaten && document.store.text() == "안녕 ",
            format!("eaten {eaten}, text {:?}", document.store.text()),
        );
    }

    // Moving to another field commits what was being typed into the first.
    if let Ok(first) = harness.document() {
        harness.type_script(&first, "dkssud");
        if let Ok(second) = harness.document() {
            harness.type_script(&second, "hello ");
            report.check(
                "focus change commits",
                first.store.text() == "안녕" && first.store.compositions.get() == 0 && second.store.text() == "hello ",
                format!("first {:?} ({} compositions), second {:?}", first.store.text(), first.store.compositions.get(), second.store.text()),
            );
        }
    }

    harness.restore();
    println!("\n{} passed, {} failed", report.passed, report.failed);
    if std::env::var_os("SEAMTYPE_LOG").is_some() {
        if let Ok(log) = std::fs::read_to_string(dir.join("seamtype.log")) {
            println!("\n--- text service log\n{log}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if report.failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
