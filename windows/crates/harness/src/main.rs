//! End-to-end checks of the text service through Windows' own Text Services Framework.
//!
//! The harness plays the app: it makes TSF documents over a text store of its own (store.rs),
//! activates Seamtype's text service on its thread the way TSF does for an input method, and
//! sends key events through TSF's keystroke manager. Keys the text service passes on are typed
//! by the "app", as a real text field would. Nothing is registered and no real keyboard input
//! or window focus is involved, so it runs on a machine in use.
//!
//!   seamtype-harness            run every scenario, print each result, exit 1 on a failure

mod store;

use std::process::ExitCode;

use seamtype::{TextService, CLSID_SEAMTYPE};
use store::Store;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITextStoreACP, ITfClientId, ITfContext, ITfDocumentMgr, ITfKeyEventSink, ITfKeystrokeMgr,
    ITfTextInputProcessorEx, ITfThreadMgr, ITfThreadMgrEventSink, ITfThreadMgrEx, TF_TMAE_NOACTIVATETIP,
};
use windows_core::{ComObject, IUnknown, Interface, Result};

#[derive(Clone)]
struct Document {
    manager: ITfDocumentMgr,
    context: ITfContext,
    store: ComObject<Store>,
}

struct Harness {
    thread_mgr: ITfThreadMgr,
    app_client: u32,
    #[allow(dead_code)]
    keystrokes: ITfKeystrokeMgr,
    tip: ITfTextInputProcessorEx,
    /// The text service's key sink, called the way TSF's keystroke manager calls it (TSF only
    /// routes keys to an input method it activated itself, from the registry).
    keys: ITfKeyEventSink,
    /// Its focus sink, told about focus changes the way TSF tells it, for the same reason.
    focus: ITfThreadMgrEventSink,
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
    fn new() -> Result<Harness> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let step = |what: &'static str| move |e: windows_core::Error| {
                println!("  setup step failed: {what}: {e}");
                e
            };
            let thread_mgr: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).map_err(step("create thread manager"))?;
            // No input method of the system's: only Seamtype's text service, activated below.
            let mut app_client = 0u32;
            thread_mgr
                .cast::<ITfThreadMgrEx>()?
                .ActivateEx(&mut app_client, TF_TMAE_NOACTIVATETIP)
                .map_err(step("activate thread manager"))?;
            let tip_client =
                thread_mgr.cast::<ITfClientId>()?.GetClientId(&CLSID_SEAMTYPE).map_err(step("client id"))?;
            let tip: ITfTextInputProcessorEx = TextService::new().into();
            let keystrokes: ITfKeystrokeMgr = thread_mgr.cast()?;
            let keys: ITfKeyEventSink = tip.cast()?;
            let focus: ITfThreadMgrEventSink = tip.cast()?;
            let harness = Harness { thread_mgr, app_client, keystrokes, tip, keys, focus, documents: Default::default() };
            harness.tip.ActivateEx(&harness.thread_mgr, tip_client, 0).map_err(step("activate text service"))?;
            Ok(harness)
        }
    }

    /// A new, focused text field.
    fn document(&self) -> Result<Document> {
        unsafe {
            let manager = self.thread_mgr.CreateDocumentMgr()?;
            let store = ComObject::new(Store::new());
            let unknown: IUnknown = store.to_interface::<ITextStoreACP>().cast()?;
            let mut context = None;
            let mut cookie = 0u32;
            manager.CreateContext(self.app_client, 0, &unknown, &mut context, &mut cookie)?;
            let context = context.expect("context");
            manager.Push(&context)?;
            let previous = self.documents.borrow().last().map(|d| d.manager.clone());
            self.thread_mgr.SetFocus(&manager)?;
            self.focus.OnSetFocus(&manager, previous.as_ref())?;
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
        let (vk, shift, typed) = press_for(ch);
        self.set_shift(shift);
        let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) };
        let wparam = WPARAM(vk.0 as usize);
        let down = LPARAM((1 | (scan << 16)) as isize);
        let up = LPARAM((1 | (scan << 16) | (0xC000_0000u32)) as isize);
        let context = &document.context;
        let eaten = unsafe {
            if vk == VK_HANGUL {
                self.keys.OnPreservedKey(context, &seamtype::GUID_PRESERVED_PAUSE).map(|b| b.as_bool()).unwrap_or(false)
            } else {
                let test = self.keys.OnTestKeyDown(context, wparam, down).map(|b| b.as_bool()).unwrap_or(false);
                test && self.keys.OnKeyDown(context, wparam, down).map(|b| b.as_bool()).unwrap_or(false)
            }
        };
        if !eaten {
            match typed {
                Typed::Text(s) => document.store.app_insert(&s),
                Typed::Newline => document.store.app_insert("\n"),
                Typed::Backspace => document.store.app_backspace(),
                Typed::Nothing => {}
            }
        }
        unsafe {
            let test = self.keys.OnTestKeyUp(context, wparam, up).map(|b| b.as_bool()).unwrap_or(false);
            if test {
                let _ = self.keys.OnKeyUp(context, wparam, up);
            }
        }
        self.set_shift(false);
        eaten
    }

    fn type_script(&self, document: &Document, script: &str) {
        for ch in script.chars() {
            self.press(document, ch);
        }
    }
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

    let harness = match Harness::new() {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL  setting up TSF: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut report = Report { passed: 0, failed: 0 };

    // `seamtype-harness <script>...`: type each script into a new field and show what it left.
    let scripts: Vec<String> = std::env::args().skip(1).collect();
    if !scripts.is_empty() {
        for script in scripts {
            if let Ok(document) = harness.document() {
                harness.type_script(&document, &script);
                println!("{script:?} → {:?} (compositions {})", document.store.text(), document.store.compositions.get());
            }
        }
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

    println!("\n{} passed, {} failed", report.passed, report.failed);
    if std::env::var_os("SEAMTYPE_LOG").is_some() {
        if let Ok(log) = std::fs::read_to_string(dir.join("seamtype.log")) {
            println!("\n--- text service log\n{log}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if report.failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
