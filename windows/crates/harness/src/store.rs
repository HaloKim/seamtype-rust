//! A text store (the app's side of TSF): plain text, one selection, and the compositions TSF
//! reports. The harness is the app; Seamtype edits this store through TSF exactly as it edits a
//! real text field.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, HWND, POINT, RECT, S_OK};
use windows::Win32::System::Com::{FORMATETC, IDataObject};
use windows::Win32::UI::TextServices::{
    ITextStoreACP, ITextStoreACPSink, ITextStoreACP_Impl, ITfCompositionView, ITfContextOwnerCompositionSink,
    ITfContextOwnerCompositionSink_Impl, ITfRange, TEXT_STORE_LOCK_FLAGS, TEXT_STORE_TEXT_CHANGE_FLAGS, TS_AE_END,
    TS_ATTRVAL, TS_E_SYNCHRONOUS, TS_IAS_QUERYONLY, TS_LF_SYNC, TS_RT_PLAIN, TS_RUNINFO, TS_SELECTIONSTYLE,
    TS_SELECTION_ACP, TS_SS_NOHIDDENTEXT, TS_SS_TRANSITORY, TS_STATUS, TS_S_ASYNC, TS_TEXTCHANGE,
};
use windows_core::{implement, IUnknown, Interface, Ref, Result, BOOL, GUID, HRESULT, PCWSTR, PWSTR};

#[derive(Default)]
struct Inner {
    text: Vec<u16>,
    selection: (i32, i32),
    sink: Option<ITextStoreACPSink>,
}

#[implement(ITextStoreACP, ITfContextOwnerCompositionSink)]
pub struct Store {
    inner: RefCell<Inner>,
    locked: Cell<u32>,
    queued: Cell<u32>,
    /// Compositions in progress (TSF tells the owner).
    pub compositions: Cell<i32>,
    /// Reports itself as transitory, as Windows' IMM32 bridge does for apps that aren't TSF-aware.
    pub transitory: Cell<bool>,
    /// The composition's extent, as TSF last reported it.
    comp: Cell<Option<(i32, i32)>>,
    /// Locks granted so far, and the one the composition began in (as the app sees it: handing
    /// over the front of a composition ends it there and begins a new one).
    locks: Cell<u32>,
    born: Cell<u32>,
    /// How long the composition was when time last passed (`tick`), None if it began since.
    settled: Cell<Option<i32>>,
    /// The composition's changes since it began (when, how long), for the app's timer to catch.
    changes: RefCell<Vec<(std::time::Instant, i32)>>,
    /// What an app like Tabby (xterm.js) got: it notes a composition's extent only on a timer
    /// after each change, and takes what the composition hands over by that extent. A
    /// composition begun and ended in one change goes in whole.
    received: RefCell<String>,
    /// Characters such an app lost.
    pub lost: Cell<usize>,
    /// Resets its field a moment after Backspace empties it, as Excel on the web does to a cell:
    /// a composition begun meanwhile is refused (what it would show is lost).
    pub resets_when_emptied: Cell<bool>,
    resetting_until: Cell<Option<std::time::Instant>>,
}

impl Store {
    pub fn new() -> Store {
        Store {
            inner: RefCell::new(Inner::default()),
            locked: Cell::new(0),
            queued: Cell::new(0),
            compositions: Cell::new(0),
            transitory: Cell::new(false),
            comp: Cell::new(None),
            locks: Cell::new(0),
            born: Cell::new(0),
            settled: Cell::new(None),
            changes: RefCell::new(Vec::new()),
            received: RefCell::new(String::new()),
            lost: Cell::new(0),
            resets_when_emptied: Cell::new(false),
            resetting_until: Cell::new(None),
        }
    }

    /// What an xterm.js-like app shows: what it received, and what is composed.
    pub fn received(&self) -> String {
        let mut out = self.received.borrow().clone();
        if let Some((s, e)) = self.comp.get() {
            out.push_str(&self.slice(s, e));
        }
        out
    }

    fn slice(&self, s: i32, e: i32) -> String {
        let inner = self.inner.borrow();
        let (s, e) = (s.max(0) as usize, (e.max(0) as usize).min(inner.text.len()));
        if s >= e { String::new() } else { String::from_utf16_lossy(&inner.text[s..e]) }
    }

    /// Time passes: the app's timers run.
    pub fn tick(&self) {
        self.settled.set(self.comp.get().map(|(s, e)| e - s));
    }

    /// The app's timer runs this long after a change (a busy app's is later).
    const APP_TIMER: std::time::Duration = std::time::Duration::from_millis(5);

    /// The extent the app's timer has noted by now: after `tick`, or a change old enough.
    fn noted(&self) -> Option<i32> {
        let caught = self.changes.borrow().iter().rev().find(|(t, _)| t.elapsed() >= Self::APP_TIMER).map(|c| c.1);
        match (self.settled.get(), caught) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    fn changed(&self) {
        if let Some((s, e)) = self.comp.get() {
            self.changes.borrow_mut().push((std::time::Instant::now(), e - s));
        }
    }

    fn handed_over(&self, s: i32, e: i32) {
        let piece = self.slice(s, e);
        let kept: String = if self.born.get() == self.locks.get() {
            piece.clone()
        } else {
            piece.chars().take(self.noted().unwrap_or(0).max(0) as usize).collect()
        };
        self.lost.set(self.lost.get() + piece.chars().count() - kept.chars().count());
        self.received.borrow_mut().push_str(&kept);
    }

    pub fn text(&self) -> String {
        String::from_utf16_lossy(&self.inner.borrow().text)
    }

    pub fn selection(&self) -> (i32, i32) {
        self.inner.borrow().selection
    }

    /// The app types `s` itself (a key the input method passed on), and tells TSF.
    pub fn app_insert(&self, s: &str) {
        self.received.borrow_mut().push_str(s);
        let (change, sink) = {
            let mut inner = self.inner.borrow_mut();
            let (start, end) = inner.selection;
            let new: Vec<u16> = s.encode_utf16().collect();
            let n = new.len() as i32;
            inner.text.splice(start as usize..end as usize, new);
            inner.selection = (start + n, start + n);
            (TS_TEXTCHANGE { acpStart: start, acpOldEnd: end, acpNewEnd: start + n }, inner.sink.clone())
        };
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.OnSelectionChange();
                let _ = sink.OnTextChange(TEXT_STORE_TEXT_CHANGE_FLAGS(0), &change);
            }
        }
    }

    /// The app deletes the character before the caret (Backspace it got).
    pub fn app_backspace(&self) {
        self.received.borrow_mut().pop();
        let (change, sink) = {
            let mut inner = self.inner.borrow_mut();
            let (start, end) = inner.selection;
            let (from, to) = if start != end { (start, end) } else if start > 0 { (start - 1, start) } else { return };
            inner.text.drain(from as usize..to as usize);
            inner.selection = (from, from);
            if inner.text.is_empty() && self.resets_when_emptied.get() {
                self.resetting_until.set(Some(std::time::Instant::now() + std::time::Duration::from_millis(10)));
            }
            (TS_TEXTCHANGE { acpStart: from, acpOldEnd: to, acpNewEnd: from }, inner.sink.clone())
        };
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.OnSelectionChange();
                let _ = sink.OnTextChange(TEXT_STORE_TEXT_CHANGE_FLAGS(0), &change);
            }
        }
    }

    /// The user clicks somewhere: the caret moves.
    pub fn app_move_caret(&self, to: i32) {
        let sink = {
            let mut inner = self.inner.borrow_mut();
            inner.selection = (to, to);
            inner.sink.clone()
        };
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.OnSelectionChange();
            }
        }
    }

    fn grant(&self, flags: u32) -> HRESULT {
        let sink = self.inner.borrow().sink.clone();
        let Some(sink) = sink else { return E_INVALIDARG };
        self.locked.set(flags);
        self.locks.set(self.locks.get() + 1);
        let result = unsafe { sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(flags & !TS_LF_SYNC)) };
        self.locked.set(0);
        match result {
            Ok(()) => S_OK,
            Err(e) => e.code(),
        }
    }
}

impl ITextStoreACP_Impl for Store_Impl {
    fn AdviseSink(&self, riid: *const GUID, punk: Ref<IUnknown>, _mask: u32) -> Result<()> {
        if unsafe { *riid } != ITextStoreACPSink::IID {
            return Err(E_INVALIDARG.into());
        }
        self.inner.borrow_mut().sink = Some(punk.ok()?.cast()?);
        Ok(())
    }

    fn UnadviseSink(&self, _punk: Ref<IUnknown>) -> Result<()> {
        self.inner.borrow_mut().sink = None;
        Ok(())
    }

    fn RequestLock(&self, flags: u32) -> Result<HRESULT> {
        if self.locked.get() != 0 {
            if flags & TS_LF_SYNC != 0 {
                return Ok(TS_E_SYNCHRONOUS);
            }
            self.queued.set(flags);
            return Ok(TS_S_ASYNC);
        }
        let result = self.grant(flags);
        let queued = self.queued.replace(0);
        if queued != 0 {
            let _ = self.grant(queued);
        }
        Ok(result)
    }

    fn GetStatus(&self) -> Result<TS_STATUS> {
        let transitory = if self.transitory.get() { TS_SS_TRANSITORY } else { 0 };
        Ok(TS_STATUS { dwDynamicFlags: 0, dwStaticFlags: TS_SS_NOHIDDENTEXT | transitory })
    }

    fn QueryInsert(&self, start: i32, end: i32, _cch: u32, result_start: *mut i32, result_end: *mut i32) -> Result<()> {
        unsafe {
            *result_start = start;
            *result_end = end;
        }
        Ok(())
    }

    fn GetSelection(&self, _index: u32, count: u32, selection: *mut TS_SELECTION_ACP, fetched: *mut u32) -> Result<()> {
        if count == 0 {
            unsafe { *fetched = 0 };
            return Ok(());
        }
        let (start, end) = self.inner.borrow().selection;
        unsafe {
            *selection = TS_SELECTION_ACP {
                acpStart: start,
                acpEnd: end,
                style: TS_SELECTIONSTYLE { ase: TS_AE_END, fInterimChar: false.into() },
            };
            *fetched = 1;
        }
        Ok(())
    }

    fn SetSelection(&self, count: u32, selection: *const TS_SELECTION_ACP) -> Result<()> {
        if count > 0 {
            let s = unsafe { &*selection };
            self.inner.borrow_mut().selection = (s.acpStart, s.acpEnd);
        }
        Ok(())
    }

    fn GetText(
        &self,
        start: i32,
        end: i32,
        plain: PWSTR,
        plain_requested: u32,
        plain_returned: *mut u32,
        run_info: *mut TS_RUNINFO,
        run_info_requested: u32,
        run_info_returned: *mut u32,
        next: *mut i32,
    ) -> Result<()> {
        let inner = self.inner.borrow();
        let len = inner.text.len() as i32;
        let end = if end < 0 || end > len { len } else { end };
        let start = start.clamp(0, len);
        let n = ((end - start).max(0) as u32).min(plain_requested) as usize;
        unsafe {
            if n > 0 && !plain.is_null() {
                std::ptr::copy_nonoverlapping(inner.text[start as usize..].as_ptr(), plain.0, n);
            }
            *plain_returned = n as u32;
            if run_info_requested > 0 && !run_info.is_null() {
                *run_info = TS_RUNINFO { uCount: n as u32, r#type: TS_RT_PLAIN };
                *run_info_returned = 1;
            } else if !run_info_returned.is_null() {
                *run_info_returned = 0;
            }
            *next = start + n as i32;
        }
        Ok(())
    }

    fn SetText(&self, _flags: u32, start: i32, end: i32, text: &PCWSTR, cch: u32) -> Result<TS_TEXTCHANGE> {
        let new: Vec<u16> = unsafe { std::slice::from_raw_parts(text.0, cch as usize) }.to_vec();
        let mut inner = self.inner.borrow_mut();
        let n = new.len() as i32;
        inner.text.splice(start as usize..end as usize, new);
        let (s0, s1) = inner.selection;
        let shift = |p: i32| if p >= end { p + n - (end - start) } else if p > start { start + n } else { p };
        inner.selection = (shift(s0), shift(s1));
        Ok(TS_TEXTCHANGE { acpStart: start, acpOldEnd: end, acpNewEnd: start + n })
    }

    fn GetFormattedText(&self, _start: i32, _end: i32) -> Result<IDataObject> {
        Err(E_NOTIMPL.into())
    }

    fn GetEmbedded(&self, _pos: i32, _service: *const GUID, _riid: *const GUID) -> Result<IUnknown> {
        Err(E_NOTIMPL.into())
    }

    fn QueryInsertEmbedded(&self, _service: *const GUID, _format: *const FORMATETC) -> Result<BOOL> {
        Ok(false.into())
    }

    fn InsertEmbedded(&self, _flags: u32, _start: i32, _end: i32, _object: Ref<IDataObject>) -> Result<TS_TEXTCHANGE> {
        Err(E_NOTIMPL.into())
    }

    fn InsertTextAtSelection(&self, flags: u32, text: &PCWSTR, cch: u32, start_out: *mut i32, end_out: *mut i32, change: *mut TS_TEXTCHANGE) -> Result<()> {
        let mut inner = self.inner.borrow_mut();
        let (start, end) = inner.selection;
        if flags & TS_IAS_QUERYONLY != 0 {
            unsafe {
                *start_out = start;
                *end_out = end;
            }
            return Ok(());
        }
        let new: Vec<u16> = if cch == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(text.0, cch as usize) }.to_vec() };
        let n = new.len() as i32;
        inner.text.splice(start as usize..end as usize, new);
        inner.selection = (start + n, start + n);
        unsafe {
            if !start_out.is_null() {
                *start_out = start;
            }
            if !end_out.is_null() {
                *end_out = start + n;
            }
            if !change.is_null() {
                *change = TS_TEXTCHANGE { acpStart: start, acpOldEnd: end, acpNewEnd: start + n };
            }
        }
        Ok(())
    }

    fn InsertEmbeddedAtSelection(&self, _flags: u32, _object: Ref<IDataObject>, _start: *mut i32, _end: *mut i32, _change: *mut TS_TEXTCHANGE) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn RequestSupportedAttrs(&self, _flags: u32, _count: u32, _attrs: *const GUID) -> Result<()> {
        Ok(())
    }

    fn RequestAttrsAtPosition(&self, _pos: i32, _count: u32, _attrs: *const GUID, _flags: u32) -> Result<()> {
        Ok(())
    }

    fn RequestAttrsTransitioningAtPosition(&self, _pos: i32, _count: u32, _attrs: *const GUID, _flags: u32) -> Result<()> {
        Ok(())
    }

    fn FindNextAttrTransition(&self, _start: i32, halt: i32, _count: u32, _attrs: *const GUID, _flags: u32, next: *mut i32, found: *mut BOOL, offset: *mut i32) -> Result<()> {
        unsafe {
            *next = halt;
            *found = false.into();
            *offset = 0;
        }
        Ok(())
    }

    fn RetrieveRequestedAttrs(&self, _count: u32, _values: *mut TS_ATTRVAL, fetched: *mut u32) -> Result<()> {
        unsafe { *fetched = 0 };
        Ok(())
    }

    fn GetEndACP(&self) -> Result<i32> {
        Ok(self.inner.borrow().text.len() as i32)
    }

    fn GetActiveView(&self) -> Result<u32> {
        Ok(1)
    }

    fn GetACPFromPoint(&self, _view: u32, _point: *const POINT, _flags: u32) -> Result<i32> {
        Err(E_NOTIMPL.into())
    }

    fn GetTextExt(&self, _view: u32, start: i32, end: i32, rect: *mut RECT, clipped: *mut BOOL) -> Result<()> {
        unsafe {
            *rect = RECT { left: 100 + start * 10, top: 100, right: 100 + end * 10, bottom: 120 };
            *clipped = false.into();
        }
        Ok(())
    }

    fn GetScreenExt(&self, _view: u32) -> Result<RECT> {
        Ok(RECT { left: 0, top: 0, right: 800, bottom: 600 })
    }

    fn GetWnd(&self, _view: u32) -> Result<HWND> {
        Ok(HWND::default())
    }
}

impl ITfContextOwnerCompositionSink_Impl for Store_Impl {
    fn OnStartComposition(&self, composition: Ref<ITfCompositionView>) -> Result<BOOL> {
        if self.resetting_until.get().is_some_and(|t| std::time::Instant::now() < t) {
            return Ok(false.into());
        }
        self.compositions.set(self.compositions.get() + 1);
        self.born.set(self.locks.get());
        self.settled.set(None);
        self.changes.borrow_mut().clear();
        self.comp.set(composition.as_ref().and_then(|c| unsafe { c.GetRange() }.ok()).and_then(|r| extent(&r)));
        self.changed();
        Ok(true.into())
    }

    fn OnUpdateComposition(&self, composition: Ref<ITfCompositionView>, range: Ref<ITfRange>) -> Result<()> {
        let new = range
            .as_ref()
            .cloned()
            .or_else(|| composition.as_ref().and_then(|c| unsafe { c.GetRange() }.ok()))
            .and_then(|r| extent(&r));
        if let (Some((s, _)), Some((s2, _))) = (self.comp.get(), new) {
            if s2 > s {
                self.handed_over(s, s2);
                self.born.set(self.locks.get());
                self.settled.set(None);
                self.changes.borrow_mut().clear();
            }
        }
        self.comp.set(new);
        self.changed();
        Ok(())
    }

    fn OnEndComposition(&self, composition: Ref<ITfCompositionView>) -> Result<()> {
        self.compositions.set(self.compositions.get() - 1);
        let last = composition.as_ref().and_then(|c| unsafe { c.GetRange() }.ok()).and_then(|r| extent(&r)).or(self.comp.get());
        if let Some((s, e)) = last {
            self.handed_over(s, e);
        }
        self.comp.set(None);
        Ok(())
    }
}

#[allow(dead_code)]
fn _check(store: &windows_core::ComObject<Store>) -> ITextStoreACP {
    store.to_interface()
}

fn extent(range: &ITfRange) -> Option<(i32, i32)> {
    let acp: windows::Win32::UI::TextServices::ITfRangeACP = range.cast().ok()?;
    let (mut start, mut length) = (0i32, 0i32);
    unsafe { acp.GetExtent(&mut start, &mut length) }.ok()?;
    Some((start, start + length))
}
