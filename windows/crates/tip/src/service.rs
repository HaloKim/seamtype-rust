//! The text service (InputController.swift): adapts TSF to `InputSession`.
//!
//! TSF makes one instance per thread that types, in every app. Each holds one session; when
//! focus moves to another text field, what was being composed goes into the field it was typed
//! in and the session starts over for the new one, as macOS activates a controller per field.
//!
//! Keys: `OnTestKeyDown` claims the keys the session will look at, and `OnKeyDown` hands them to
//! it inside a synchronous edit session; the session's answer is whether the key was eaten.
//! Keys it passes on (Space after a word, punctuation, shortcuts) reach the app after the word
//! went in, as with the system's Korean input. Keys that can't change the text while nothing
//! is composed (arrows, Tab, shortcuts) are seen without claiming them, so the session still
//! hears that the sentence context ended.

use std::cell::RefCell;
use std::rc::Rc;

use seamtype_core::{InputSession, Key, TextClient, TextRange};
use windows::Win32::Foundation::{E_UNEXPECTED, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Variant::VariantClear;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_HANGUL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, IEnumTfDisplayAttributeInfo, ITfCategoryMgr, ITfDisplayAttributeProvider, ITfCompartmentMgr, ITfComposition,
    ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfDisplayAttributeInfo, ITfDisplayAttributeProvider_Impl,
    ITfDocumentMgr, ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr, ITfLangBarItemButton, ITfLangBarItemMgr, ITfSource,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl, ITfThreadMgr,
    ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl, GUID_COMPARTMENT_EMPTYCONTEXT,
    GUID_COMPARTMENT_KEYBOARD_DISABLED, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, TF_INVALID_COOKIE, TF_PRESERVEDKEY,
};
use windows_core::{implement, ComObject, Error, IUnknown, IUnknownImpl, Interface, Ref, Result, BOOL, GUID};

use crate::client::{variant_as_i32, variant_i4, TsfClient};
use crate::display::{AttributeEnum, InputAttribute};
use crate::langbar::LangBarButton;
use crate::{edit, keys, runtime, strings, GUID_DISPLAY_ATTRIBUTE_INPUT, GUID_PRESERVED_PAUSE};

#[derive(Default)]
pub struct State {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    thread_mgr_cookie: Option<u32>,
    composition: Option<ITfComposition>,
    session: Option<InputSession>,
    generation: u64,
    atom: u32,
    langbar: Option<ComObject<LangBarButton>>,
    /// Keys whose key-down was eaten: their key-up is eaten too.
    eaten: Vec<u32>,
    pause_key: bool,
}

#[implement(ITfTextInputProcessorEx, ITfKeyEventSink, ITfThreadMgrEventSink, ITfCompositionSink, ITfDisplayAttributeProvider)]
pub struct TextService {
    state: RefCell<State>,
}

impl Default for TextService {
    fn default() -> Self {
        TextService::new()
    }
}

impl TextService {
    pub fn new() -> TextService {
        TextService { state: RefCell::new(State::default()) }
    }

    pub fn is_paused(&self) -> bool {
        self.state.try_borrow().map(|s| s.session.as_ref().map(|s| s.paused()).unwrap_or(false)).unwrap_or(false)
    }

    /// The session, rebuilt (after committing what was being typed) when a setting changed.
    fn refresh_session(state: &mut State, client: Option<&mut dyn TextClient>) {
        // The runtime's lock isn't held while the old session commits: that calls into TSF.
        let generation = runtime::get().generation;
        if state.session.is_some() && state.generation == generation {
            return;
        }
        if let Some(old) = state.session.as_mut() {
            old.commit(client);
        }
        let runtime = runtime::get();
        state.session = Some(runtime.make_session(Some(runtime::app_id())));
        state.generation = runtime.generation;
    }

    /// The field the composition was in went away (its context popped, its document gone):
    /// forget the composition without touching that field again.
    fn drop_composition_if(&self, gone: impl Fn(&ITfContext) -> bool) {
        let Ok(mut state) = self.state.try_borrow_mut() else { return };
        let context = state
            .composition
            .as_ref()
            .and_then(|c| unsafe { c.GetRange().ok() })
            .and_then(|r| unsafe { r.GetContext().ok() });
        if context.as_ref().map(|c| gone(c)).unwrap_or(false) {
            state.composition = None;
            if let Some(session) = state.session.as_mut() {
                session.abandon();
            }
            crate::log!("the composition's field went away");
        }
    }
}

/// What a key needs from the session.
enum KeyAction {
    /// Not Seamtype's: the app gets it untouched.
    Ignore,
    /// Can't change the text: the session hears it, the app gets it.
    Idle(Key),
    /// Handled inside an edit session.
    Edit(Key),
}

/// A client for keys that can't touch the text (`KeyAction::Idle`).
struct NullClient;

impl TextClient for NullClient {
    fn set_marked_text(&mut self, _: &str) {}
    fn insert_text(&mut self, _: &str, _: Option<TextRange>) {}
    fn selected_range(&self) -> TextRange {
        TextRange::NONE
    }
    fn marked_range(&self) -> TextRange {
        TextRange::NONE
    }
    fn text_in(&self, _: TextRange) -> Option<String> {
        None
    }
}

fn compartment_on(manager: &ITfCompartmentMgr, guid: &GUID) -> bool {
    unsafe {
        let Ok(compartment) = manager.GetCompartment(guid) else { return false };
        let Ok(mut value) = compartment.GetValue() else { return false };
        let on = variant_as_i32(&value).map(|v| v != 0).unwrap_or(false);
        let _ = VariantClear(&mut value);
        on
    }
}

/// The app turned input methods off for this field (password fields, read-only views…).
fn keyboard_disabled(context: &ITfContext) -> bool {
    let Ok(manager) = context.cast::<ITfCompartmentMgr>() else { return false };
    compartment_on(&manager, &GUID_COMPARTMENT_KEYBOARD_DISABLED) || compartment_on(&manager, &GUID_COMPARTMENT_EMPTYCONTEXT)
}

fn same_object(a: &impl Interface, b: &impl Interface) -> bool {
    match (a.cast::<IUnknown>(), b.cast::<IUnknown>()) {
        (Ok(a), Ok(b)) => a.as_raw() == b.as_raw(),
        _ => false,
    }
}

fn pause_key() -> TF_PRESERVEDKEY {
    TF_PRESERVEDKEY { uVKey: VK_HANGUL.0 as u32, uModifiers: 0 }
}

impl TextService_Impl {
    fn client_id(&self) -> u32 {
        self.state.try_borrow().map(|s| s.client_id).unwrap_or(0)
    }

    /// Runs `f` with the state and a client over `context` in a synchronous edit session.
    /// None when TSF couldn't grant one right now.
    fn edit<R: 'static>(&self, context: &ITfContext, f: impl FnOnce(&mut State, &mut TsfClient) -> R + 'static) -> Option<R> {
        let this = self.to_object();
        let ctx = context.clone();
        let sink: ITfCompositionSink = self.to_interface();
        let out: Rc<RefCell<Option<R>>> = Rc::new(RefCell::new(None));
        let result = out.clone();
        let ran = edit::sync_write(context, self.client_id(), move |ec| {
            let mut guard = this.state.try_borrow_mut().map_err(|_| Error::from(E_UNEXPECTED))?;
            let state = &mut *guard;
            let mut composition = state.composition.take();
            let atom = state.atom;
            let value = {
                let mut client = TsfClient { ec, context: &ctx, composition: &mut composition, sink: &sink, atom };
                f(state, &mut client)
            };
            state.composition = composition;
            *result.borrow_mut() = Some(value);
            Ok(())
        });
        match ran {
            Ok(true) => out.borrow_mut().take(),
            Ok(false) => {
                crate::log!("edit session not granted");
                None
            }
            Err(e) => {
                crate::log!("edit session failed: {e}");
                None
            }
        }
    }

    /// The document the composition is in, if there is one.
    fn composition_context(&self) -> Option<ITfContext> {
        let state = self.state.try_borrow().ok()?;
        let composition = state.composition.as_ref()?;
        unsafe { composition.GetRange().ok()?.GetContext().ok() }
    }

    /// Commits what is being composed into the field it was typed in, or, when that field can't
    /// be edited now, forgets it (the field keeps what it showed).
    fn finish_composition(&self) {
        if let Some(context) = self.composition_context() {
            crate::log!("finishing the composition");
            let done = self.edit(&context, |state, client| {
                if let Some(session) = state.session.as_mut() {
                    session.commit_for_new_context(Some(client));
                }
            });
            crate::log!("finished: {}", done.is_some());
            if done.is_some() && self.state.borrow().composition.is_none() {
                return;
            }
        }
        let mut state = self.state.borrow_mut();
        if let Some(session) = state.session.as_mut() {
            if session.is_composing() {
                session.abandon();
            }
        }
        state.composition = None;
    }

    fn classify(&self, context: &ITfContext, vk: u32) -> KeyAction {
        if keyboard_disabled(context) {
            return KeyAction::Ignore;
        }
        let Some(key) = keys::key_for(vk, &keys::modifiers()) else { return KeyAction::Ignore };
        runtime::get().refresh(false);
        let Ok(mut state) = self.state.try_borrow_mut() else { return KeyAction::Ignore };
        if state.thread_mgr.is_none() {
            return KeyAction::Ignore;
        }
        if !state.session.as_ref().map(|s| s.is_composing()).unwrap_or(false) {
            TextService::refresh_session(&mut state, None);
        }
        let Some(session) = state.session.as_ref() else { return KeyAction::Ignore };
        if session.passes_through(key) {
            return KeyAction::Ignore;
        }
        let idle = !session.is_composing() && !session.has_flip_window() && !matches!(key, Key::Letter(_) | Key::Printable(_));
        if idle { KeyAction::Idle(key) } else { KeyAction::Edit(key) }
    }

    fn handle_idle(&self, key: Key) {
        if let Ok(mut state) = self.state.try_borrow_mut() {
            if let Some(session) = state.session.as_mut() {
                session.handle(key, &mut NullClient);
                crate::log!("key {} idle", key.kind());
            }
        }
    }

    fn handle_key(&self, context: &ITfContext, vk: u32, test: bool) -> bool {
        match self.classify(context, vk) {
            KeyAction::Ignore => false,
            KeyAction::Idle(key) => {
                self.handle_idle(key);
                false
            }
            KeyAction::Edit(_) if test => true,
            KeyAction::Edit(key) => {
                // A composition left in another field (focus moved without telling) goes in first.
                if let Some(other) = self.composition_context() {
                    if !same_object(&other, context) {
                        self.finish_composition();
                    }
                }
                let handled = self
                    .edit(context, move |state, client| {
                        TextService::refresh_session(state, Some(client));
                        state.session.as_mut().map(|s| s.handle(key, client)).unwrap_or(false)
                    })
                    .unwrap_or(false);
                crate::log!("key {} handled={handled}", key.kind());
                handled
            }
        }
    }

    pub fn toggle_pause(&self) {
        let context = self.composition_context().or_else(|| self.focused_context());
        let composing = self.state.borrow().session.as_ref().map(|s| s.is_composing()).unwrap_or(false);
        let toggled = match (&context, composing) {
            (Some(context), true) => self
                .edit(context, |state, client| {
                    if let Some(session) = state.session.as_mut() {
                        session.toggle_pause(Some(client));
                    }
                })
                .is_some(),
            _ => false,
        };
        if !toggled {
            let mut state = self.state.borrow_mut();
            if let Some(session) = state.session.as_mut() {
                if session.is_composing() {
                    session.abandon();
                }
                session.toggle_pause(None);
            }
            state.composition = None;
        }
        crate::log!("paused={}", self.is_paused());
        self.update_langbar();
    }

    fn focused_context(&self) -> Option<ITfContext> {
        let state = self.state.borrow();
        let thread_mgr = state.thread_mgr.as_ref()?;
        unsafe { thread_mgr.GetFocus().ok()?.GetTop().ok() }
    }

    pub fn update_langbar(&self) {
        let button = self.state.try_borrow().ok().and_then(|s| s.langbar.clone());
        if let Some(button) = button {
            button.changed();
        }
    }

    /// The app this text service types into, for the menu's per-app settings.
    pub fn app(&self) -> String {
        runtime::app_id().to_string()
    }

    /// Applies a settings change at once (the menu): a fresh session after committing.
    pub fn settings_changed(&self) {
        let context = self.composition_context();
        if let Some(context) = context {
            self.edit(&context, |state, client| TextService::refresh_session(state, Some(client)));
        } else if let Ok(mut state) = self.state.try_borrow_mut() {
            TextService::refresh_session(&mut state, None);
        }
        self.update_langbar();
    }

    pub fn last_learned(&self) -> Option<String> {
        runtime::get().engine.last_learned()
    }

    fn activate(&self, thread_mgr: &ITfThreadMgr, client_id: u32) -> Result<()> {
        {
            let mut state = self.state.borrow_mut();
            state.thread_mgr = Some(thread_mgr.clone());
            state.client_id = client_id;
        }
        unsafe {
            let source: ITfSource = thread_mgr.cast()?;
            let sink: ITfThreadMgrEventSink = self.to_interface();
            let cookie = source.AdviseSink(&ITfThreadMgrEventSink::IID, &sink)?;
            self.state.borrow_mut().thread_mgr_cookie = (cookie != TF_INVALID_COOKIE).then_some(cookie);

            let keystrokes: ITfKeystrokeMgr = thread_mgr.cast()?;
            let key_sink: ITfKeyEventSink = self.to_interface();
            if let Err(e) = keystrokes.AdviseKeyEventSink(client_id, &key_sink, true) {
                crate::log!("could not receive keys: {e}");
            }
            if runtime::get().settings.right_command_pause {
                let description: Vec<u16> = strings::get().pause_key.encode_utf16().collect();
                if keystrokes.PreserveKey(client_id, &GUID_PRESERVED_PAUSE, &pause_key(), &description).is_ok() {
                    self.state.borrow_mut().pause_key = true;
                }
            }

            if let Ok(categories) = CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER) {
                if let Ok(atom) = categories.RegisterGUID(&GUID_DISPLAY_ATTRIBUTE_INPUT) {
                    self.state.borrow_mut().atom = atom;
                }
            }

            // Input methods are "open" for this thread (some apps look before sending keys).
            if let Ok(global) = thread_mgr.GetGlobalCompartment() {
                if let Ok(open) = global.GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) {
                    let _ = open.SetValue(client_id, &variant_i4(1));
                }
            }
        }

        if let Ok(items) = thread_mgr.cast::<ITfLangBarItemMgr>() {
            let button = ComObject::new(LangBarButton::new(self.to_object()));
            let item: ITfLangBarItemButton = button.to_interface();
            if unsafe { items.AddItem(&item) }.is_ok() {
                self.state.borrow_mut().langbar = Some(button);
            }
        }

        runtime::get().refresh(true);
        {
            let mut state = self.state.borrow_mut();
            TextService::refresh_session(&mut state, None);
        }
        crate::log!("activated");
        Ok(())
    }

    fn deactivate(&self) -> Result<()> {
        self.finish_composition();
        let mut state = self.state.borrow_mut();
        if let Some(thread_mgr) = state.thread_mgr.take() {
            unsafe {
                if let Ok(keystrokes) = thread_mgr.cast::<ITfKeystrokeMgr>() {
                    let _ = keystrokes.UnadviseKeyEventSink(state.client_id);
                    if state.pause_key {
                        let _ = keystrokes.UnpreserveKey(&GUID_PRESERVED_PAUSE, &pause_key());
                    }
                }
                if let (Ok(source), Some(cookie)) = (thread_mgr.cast::<ITfSource>(), state.thread_mgr_cookie) {
                    let _ = source.UnadviseSink(cookie);
                }
                if let Some(button) = state.langbar.take() {
                    if let Ok(items) = thread_mgr.cast::<ITfLangBarItemMgr>() {
                        let item: ITfLangBarItemButton = button.to_interface();
                        let _ = items.RemoveItem(&item);
                    }
                    button.detach();
                }
            }
        }
        if let Some(converter) = runtime::get().converter.as_ref() {
            converter.lock().unwrap_or_else(|e| e.into_inner()).save();
        }
        state.thread_mgr_cookie = None;
        state.pause_key = false;
        state.session = None;
        state.composition = None;
        state.eaten.clear();
        crate::log!("deactivated");
        Ok(())
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, thread_mgr: Ref<ITfThreadMgr>, client_id: u32) -> Result<()> {
        crate::guard(|| self.activate(thread_mgr.ok()?, client_id))
    }

    fn Deactivate(&self) -> Result<()> {
        crate::guard(|| self.deactivate())
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(&self, thread_mgr: Ref<ITfThreadMgr>, client_id: u32, _flags: u32) -> Result<()> {
        crate::guard(|| self.activate(thread_mgr.ok()?, client_id))
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, _foreground: BOOL) -> Result<()> {
        Ok(())
    }

    fn OnTestKeyDown(&self, context: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        crate::guard(|| Ok(context.as_ref().map(|c| self.handle_key(c, wparam.0 as u32, true)).unwrap_or(false).into()))
    }

    fn OnKeyDown(&self, context: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        crate::guard(|| {
            let vk = wparam.0 as u32;
            let eaten = context.as_ref().map(|c| self.handle_key(c, vk, false)).unwrap_or(false);
            if eaten {
                if let Ok(mut state) = self.state.try_borrow_mut() {
                    if !state.eaten.contains(&vk) {
                        state.eaten.push(vk);
                    }
                }
            }
            self.update_langbar();
            Ok(eaten.into())
        })
    }

    fn OnTestKeyUp(&self, _context: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        crate::guard(|| {
            let eaten = self.state.try_borrow().map(|s| s.eaten.contains(&(wparam.0 as u32))).unwrap_or(false);
            Ok(eaten.into())
        })
    }

    fn OnKeyUp(&self, _context: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        crate::guard(|| {
            let Ok(mut state) = self.state.try_borrow_mut() else { return Ok(false.into()) };
            let vk = wparam.0 as u32;
            let eaten = state.eaten.contains(&vk);
            state.eaten.retain(|&k| k != vk);
            Ok(eaten.into())
        })
    }

    fn OnPreservedKey(&self, _context: Ref<ITfContext>, guid: *const GUID) -> Result<BOOL> {
        crate::guard(|| {
            if guid.is_null() || unsafe { *guid } != GUID_PRESERVED_PAUSE {
                return Ok(false.into());
            }
            if !runtime::get().settings.right_command_pause {
                return Ok(false.into());
            }
            self.toggle_pause();
            Ok(true.into())
        })
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(&self, _document: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(&self, document: Ref<ITfDocumentMgr>) -> Result<()> {
        crate::guard(|| {
            if let Some(document) = document.as_ref() {
                self.drop_composition_if(|c| unsafe { c.GetDocumentMgr() }.map(|d| same_object(&d, document)).unwrap_or(true));
            }
            Ok(())
        })
    }

    fn OnSetFocus(&self, _focus: Ref<ITfDocumentMgr>, _previous: Ref<ITfDocumentMgr>) -> Result<()> {
        crate::guard(|| {
            crate::log!("focus changed");
            self.finish_composition();
            runtime::get().refresh(true);
            if let Ok(mut state) = self.state.try_borrow_mut() {
                TextService::refresh_session(&mut state, None);
                if let Some(session) = state.session.as_mut() {
                    session.activate(Some(runtime::app_id()));
                }
                state.eaten.clear();
            }
            self.update_langbar();
            Ok(())
        })
    }

    fn OnPushContext(&self, _context: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, context: Ref<ITfContext>) -> Result<()> {
        crate::guard(|| {
            if let Some(popped) = context.as_ref() {
                self.drop_composition_if(|c| same_object(c, popped));
            }
            Ok(())
        })
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    /// The app ended the composition (a click elsewhere, its own commit): the word goes in as
    /// decided, and the next word starts a new sentence.
    fn OnCompositionTerminated(&self, ec: u32, composition: Ref<ITfComposition>) -> Result<()> {
        crate::guard(|| {
            let Ok(mut guard) = self.state.try_borrow_mut() else { return Ok(()) };
            let state = &mut *guard;
            let ours = match (state.composition.as_ref(), composition.as_ref()) {
                (Some(mine), Some(theirs)) => same_object(mine, theirs),
                _ => false,
            };
            if !ours {
                return Ok(());
            }
            let context = unsafe { state.composition.as_ref().and_then(|c| c.GetRange().ok()).and_then(|r| r.GetContext().ok()) };
            let mut terminated = state.composition.take();
            if let Some(context) = context {
                let sink: ITfCompositionSink = self.to_interface();
                let mut client = TsfClient { ec, context: &context, composition: &mut terminated, sink: &sink, atom: state.atom };
                if let Some(session) = state.session.as_mut() {
                    session.commit_for_new_context(Some(&mut client));
                }
            } else if let Some(session) = state.session.as_mut() {
                session.abandon();
            }
            state.composition = None;
            crate::log!("composition terminated by the app");
            Ok(())
        })
    }
}

impl ITfDisplayAttributeProvider_Impl for TextService_Impl {
    fn EnumDisplayAttributeInfo(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        Ok(AttributeEnum { index: Default::default() }.into())
    }

    fn GetDisplayAttributeInfo(&self, guid: *const GUID) -> Result<ITfDisplayAttributeInfo> {
        if !guid.is_null() && unsafe { *guid } == GUID_DISPLAY_ATTRIBUTE_INPUT {
            Ok(InputAttribute.into())
        } else {
            Err(windows::Win32::Foundation::E_INVALIDARG.into())
        }
    }
}


