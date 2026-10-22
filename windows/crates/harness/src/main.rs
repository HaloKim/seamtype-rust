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

    /// Focus goes to a document that takes no text and straight back (Chrome does this while
    /// Excel on the web is edited).
    fn bounce_focus(&self, document: &Document) -> Result<()> {
        unsafe {
            let nothing = self.thread_mgr.CreateDocumentMgr()?;
            for (to, from) in [(&nothing, &document.manager), (&document.manager, &nothing)] {
                self.thread_mgr.SetFocus(to)?;
                if let Route::Direct { focus, .. } = &self.route {
                    focus.OnSetFocus(to, from)?;
                }
            }
            Ok(())
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
        // Time passes between key presses (unless typing faster than the app's timers).
        if !FAST.load(std::sync::atomic::Ordering::SeqCst) {
            document.store.tick();
        }
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
        // (claimed by the key test, eaten by the key itself)
        let deliver = || unsafe {
            match &self.route {
                Route::Direct { keys, .. } if vk == VK_HANGUL => {
                    let eaten = keys.OnPreservedKey(context, &seamtype::GUID_PRESERVED_PAUSE).map(|b| b.as_bool()).unwrap_or(false);
                    (eaten, eaten)
                }
                Route::Direct { keys, .. } => {
                    let test = keys.OnTestKeyDown(context, wparam, down).map(|b| b.as_bool()).unwrap_or(false);
                    (test, test && keys.OnKeyDown(context, wparam, down).map(|b| b.as_bool()).unwrap_or(false))
                }
                Route::Tsf { .. } => {
                    let test = self.keystrokes.TestKeyDown(wparam, down).map(|b| b.as_bool()).unwrap_or(false);
                    (test, test && self.keystrokes.KeyDown(wparam, down).map(|b| b.as_bool()).unwrap_or(false))
                }
            }
        };
        let (mut claimed, mut eaten) = deliver();
        for _ in 1..deliveries {
            let (c, e) = deliver();
            claimed |= c;
            eaten |= e;
        }
        // Windows' IMM32 bridge (a transitory document) loses a key the input method claimed,
        // even when it then lets it go.
        if claimed && document.store.transitory.get() {
            eaten = true;
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
        self.replay(document);
        eaten
    }

    /// Keys the text service sent: the app gets them (Backspaces, then the marker comes back).
    /// The marker coming back may send more (a longer change goes on one character per edit).
    fn replay(&self, document: &Document) {
        loop {
            let injected: Vec<seamtype::Injection> = std::mem::take(&mut *INJECTED.lock().unwrap());
            if injected.is_empty() {
                break;
            }
            for injection in injected {
                match injection {
                    seamtype::Injection::Erase(count) => {
                        ERASES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        // Markers sent earlier for later come back first (Chrome: 연세대).
                        let early: Vec<_> = std::mem::take(&mut *DEFERRED.lock().unwrap());
                        for _ in early {
                            self.marker(document);
                        }
                        for _ in 0..count {
                            document.store.app_backspace();
                        }
                        self.marker(document);
                    }
                    seamtype::Injection::Key(k) if k == VK_BACK.0 as u32 => document.store.app_backspace(),
                    seamtype::Injection::Key(k) if k == VK_RETURN.0 as u32 => document.store.app_insert("\n"),
                    seamtype::Injection::Key(_) => {}
                    seamtype::Injection::Continue => {
                        let due = std::time::Instant::now() + seamtype::CONTINUE_DELAY;
                        if FAST.load(std::sync::atomic::Ordering::SeqCst) {
                            // Comes back later, among the keys still being typed.
                            DEFERRED.lock().unwrap().push(due);
                        } else {
                            std::thread::sleep(seamtype::CONTINUE_DELAY);
                            document.store.tick();
                            self.marker(document);
                        }
                    }
                }
            }
        }
    }

    /// Markers sent for later whose moment has come.
    fn press_marker_if_due(&self, document: &Document) {
        let now = std::time::Instant::now();
        let due: Vec<_> = {
            let mut deferred = DEFERRED.lock().unwrap();
            let (due, later): (Vec<_>, Vec<_>) = deferred.drain(..).partition(|t| *t <= now);
            *deferred = later;
            due
        };
        for _ in due {
            document.store.tick();
            self.marker(document);
            self.replay(document);
        }
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

/// Markers the text service sent for later (`Injection::Continue`), by when they come back.
static DEFERRED: std::sync::Mutex<Vec<std::time::Instant>> = std::sync::Mutex::new(Vec::new());

/// Keys come faster than an xterm.js-like app's timers run.
static FAST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Keys the text service asked to send, instead of real input.
static INJECTED: std::sync::Mutex<Vec<seamtype::Injection>> = std::sync::Mutex::new(Vec::new());
/// How many times characters the app already had were erased.
static ERASES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Whether IMM32 apps compose in the app (`imeImmInline` in the run's settings.json).
fn set_imm_inline(dir: &std::path::Path, on: bool) {
    let _ = std::fs::write(dir.join("settings.json"), format!("{{\"imeImmInline\": {on}}}\n"));
}

fn inject(injection: seamtype::Injection) {
    INJECTED.lock().unwrap().push(injection);
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
    seamtype::set_popup_headless(true);
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
        ("fork ", "fork "),
        ("dkssud fork ", "안녕 fork "),
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
        ("dkssud  gksrmf", "안녕  한글"),
        ("dkssud ⌫⌫gksrmf ", "안한글 "),
        ("hello⌫⌫p ", "help "),
        ("dhsmf 3tl", "오늘 3시"),
        ("dho?", "왜?"),
        ("fork ", "fork "),
        ("dkssud fork ", "안녕 fork "),
        ("hello fork ", "hello fork "),
        ("fork", "fork"),
        ("claude ", "claude "),
        ("claude eh ", "claude eh "),
        ("dkssud claude ", "안녕 claude "),
    ];
    // Both ways of typing in an IMM32 app: the word held in Seamtype's window until it ends (the
    // default), and composed in the app one character at a time (imeImmInline).
    for inline in [false, true] {
        set_imm_inline(&dir, inline);
        let mode = if inline { "imm" } else { "imm held" };
        for (script, expected) in imm_scripts {
            // An Esc correction made in both passes would be learned (twice is remembered).
            if inline && script.contains('⎋') {
                continue;
            }
            let Ok(document) = harness.document_with(true) else { continue };
            let mut longest = 0usize;
            for ch in script.chars() {
                harness.press(&document, ch);
                longest = longest.max(document.store.compositions.get().max(0) as usize);
            }
            // A word still being typed shows in the app, or held in Seamtype's window.
            let held = seamtype::popup_shown().unwrap_or_default();
            let text = document.store.text() + &held;
            let received = document.store.received() + &held;
            report.check(
                &format!("{mode}: {script}"),
                text == *expected && longest <= 1 && received == *expected,
                format!("got {text:?}, an xterm-like app {received:?}, expected {expected:?}"),
            );
        }

        // Typed faster than Tabby's timers (xterm.js): a character handed over too soon after another
        // was lost (claude came out clade). Steps wait for the timer, keys or not.
        FAST.store(true, std::sync::atomic::Ordering::SeqCst);
        for (script, expected) in [
            ("claude ", "claude "),
            ("fork ", "fork "),
            ("dkssud claude ", "안녕 claude "),
            ("Hello, World! ", "Hello, World! "),
            ("dlrjs githubdp commitgkf rjdi.", "이건 github에 commit할 거야."),
            ("dustpeo ", "연세대 "),
        ] {
            let Ok(document) = harness.document_with(true) else { continue };
            for ch in script.chars() {
                harness.press_marker_if_due(&document);
                harness.press(&document, ch);
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
            // Then the app catches up and the last steps go in.
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(35));
                harness.press_marker_if_due(&document);
            }
            let (text, received) = (document.store.text(), document.store.received());
            report.check(
                &format!("{mode} fast: {script}"),
                text == expected && received == expected,
                format!("got {text:?}, an xterm-like app {received:?}, expected {expected:?}"),
            );
        }
        FAST.store(false, std::sync::atomic::Ordering::SeqCst);

        // Excel on the web moves focus off the cell and back on every Backspace: the word goes on
        // (rod, English, then rodl, 개이, erased and typed again: only the last letter was left).
        if let Ok(document) = harness.document_with(true) {
            for ch in "rodls ".chars() {
                harness.press(&document, ch);
                let _ = harness.bounce_focus(&document);
            }
            let text = document.store.text();
            report.check(&format!("{mode}: focus off and back"), text == "개인 ", format!("got {text:?}"));
        }

        // Excel on the web also resets a cell Backspace empties, and a composition begun straight
        // after was lost (개인적인 left 적인).
        for (script, expected) in [("rodls ", "개인 "), ("rodlswjrdls ", "개인적인 ")] {
            let Ok(document) = harness.document_with(true) else { continue };
            document.store.resets_when_emptied.set(true);
            harness.type_script(&document, script);
            let text = document.store.text();
            report.check(&format!("{mode}: field reset when emptied: {script}"), text == expected, format!("got {text:?}"));
        }
    }

    // The start of a Korean word that could be English (an, ant → 무슨) isn't handed to the app
    // while the engine is unsure, so nothing has to be erased again.
    if let Ok(document) = harness.document_with(true) {
        let before = ERASES.load(std::sync::atomic::Ordering::SeqCst);
        harness.type_script(&document, "dkssud antms ");
        let erased = ERASES.load(std::sync::atomic::Ordering::SeqCst) - before;
        let text = document.store.text();
        report.check("imm: unsure english start waits", text == "안녕 무슨 " && erased == 0, format!("got {text:?}, {erased} erasures"));
    }
    set_imm_inline(&dir, false);

    // A held word: the app has nothing of it until it ends, Seamtype's window shows it.
    if let Ok(document) = harness.document_with(true) {
        harness.type_script(&document, "dkssud");
        let (typing, shown) = (document.store.text(), seamtype::popup_shown());
        harness.type_script(&document, " ");
        let (text, after) = (document.store.text(), seamtype::popup_shown());
        report.check(
            "imm held: the word shows in the window",
            typing.is_empty() && shown.as_deref() == Some("안녕") && text == "안녕 " && after.is_none(),
            format!("field {typing:?} window {shown:?} → field {text:?} window {after:?}"),
        );
    }

    // Moving to another field puts the held word in the first.
    if let Ok(first) = harness.document_with(true) {
        harness.type_script(&first, "dkssud");
        if let Ok(second) = harness.document_with(true) {
            harness.type_script(&second, "hello ");
            report.check(
                "imm held: focus change puts the word in",
                first.store.text() == "안녕" && second.store.text() == "hello " && seamtype::popup_shown().is_none(),
                format!("first {:?}, second {:?}", first.store.text(), second.store.text()),
            );
        }
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
