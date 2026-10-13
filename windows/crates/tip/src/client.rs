//! `TextClient` over a TSF document, valid for one edit session (IMKClient in InputController.swift).
//!
//! Marked text is a TSF composition: started at the selection, its text replaced as the word
//! grows, underlined through the display attribute property, and ended when the word is
//! committed. Positions are ACP (UTF-16) offsets, read and set through ITfRangeACP; a text store
//! without them reports no selection, so Esc after a commit simply goes to the app there.
//!
//! Apps that aren't TSF-aware get text through Windows' IMM32 bridge (CUAS), which under a Korean
//! input method follows Korean IME rules: only the composition's last character (an interim
//! character) is composed, and everything before it goes to the app as typed. There a word is
//! composed one character at a time (`ImmWord`): the characters the app already has are never
//! sent again, a Backspace over one of them is left to the app, and when they have to change
//! (Esc to another reading, a split decided late) the app erases them first.

use std::mem::ManuallyDrop;

use seamtype_core::{TextClient, TextRange};

use crate::runtime::{Caret, ImeOptions};
use windows::Win32::System::Variant::{VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfContext, ITfContextComposition, ITfInsertAtSelection, ITfRange, ITfRangeACP,
    GUID_PROP_ATTRIBUTE, TF_AE_NONE, TF_ANCHOR_END, TF_ANCHOR_START, TF_DEFAULT_SELECTION, TF_IAS_QUERYONLY, TF_SELECTION,
    TF_SELECTIONSTYLE,
};
use windows_core::{Interface, Result};

pub struct TsfClient<'a> {
    pub ec: u32,
    pub context: &'a ITfContext,
    pub composition: &'a mut Option<ITfComposition>,
    pub sink: &'a ITfCompositionSink,
    /// The display attribute atom for marked text (0: none registered).
    pub atom: u32,
    pub ime: ImeOptions,
    /// The word's share an IMM32 app already has (Some in its documents, which TSF calls transitory).
    pub imm: Option<&'a mut ImmWord>,
    /// The key being handled is Backspace.
    pub backspace: bool,
}

/// What an IMM32 app already has of the word being composed (see the module comment).
#[derive(Default, Debug)]
pub struct ImmWord {
    /// The characters of the word the app has taken as typed.
    pub delivered: String,
    /// Characters the app must erase before the word can go on: erased by injected Backspaces,
    /// then this is done (when Seamtype's marker key comes back after them).
    pub pending: Option<Pending>,
    /// The Backspace being handled goes to the app, which erases one delivered character.
    pub pass_key: bool,
    /// When the character now composed began, for the app (a new composition, or the rest of
    /// one whose front was handed over; see `too_soon`).
    pub last_step: Option<std::time::Instant>,
    /// Marker keys sent and not back yet. The word goes on only when the last one is back: an
    /// earlier one (a step's, sent with a delay) can come back before Backspaces sent after it,
    /// and going on then had the app erase the wrong character (연세대 typed 세대 in Chrome).
    pub markers: u32,
    /// What came while a commit was still going in, in order: it follows the commit.
    pub queue: Vec<Queued>,
}

#[derive(Clone, Debug)]
pub enum Queued {
    /// Text typed as finished (a word's commit, a space).
    Commit(String),
    /// What the next word shows while it is typed (the latest replaces the one before).
    Show(String),
}

#[derive(Clone, Debug)]
pub struct Pending {
    pub erase: usize,
    pub text: String,
    /// `text` ends the word (a commit) rather than being what is still composed.
    pub commit: bool,
}

/// How many leading characters `a` and `b` share.
fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

pub fn variant_i4(value: i32) -> VARIANT {
    variant(VT_I4, VARIANT_0_0_0 { lVal: value })
}

fn variant(vt: VARENUM, value: VARIANT_0_0_0) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 { vt, wReserved1: 0, wReserved2: 0, wReserved3: 0, Anonymous: value }),
        },
    }
}

/// The value of a VT_I4 variant (compartments and properties hold those).
pub fn variant_as_i32(v: &VARIANT) -> Option<i32> {
    unsafe {
        let inner = &v.Anonymous.Anonymous;
        (inner.vt == VT_I4).then(|| inner.Anonymous.lVal)
    }
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl TsfClient<'_> {
    fn insert_point(&self) -> Result<ITfRange> {
        let insert: ITfInsertAtSelection = self.context.cast()?;
        unsafe { insert.InsertTextAtSelection(self.ec, TF_IAS_QUERYONLY, &[]) }
    }

    fn select(&self, range: ITfRange, interim: bool) -> Result<()> {
        unsafe {
            let selection = TF_SELECTION {
                range: ManuallyDrop::new(Some(range)),
                style: TF_SELECTIONSTYLE { ase: TF_AE_NONE, fInterimChar: interim.into() },
            };
            let result = self.context.SetSelection(self.ec, std::slice::from_ref(&selection));
            drop(ManuallyDrop::into_inner(selection.range));
            result
        }
    }

    /// Puts the caret (an empty selection) at the end of `range`.
    fn caret_after(&self, range: &ITfRange) -> Result<()> {
        unsafe {
            let caret = range.Clone()?;
            caret.Collapse(self.ec, TF_ANCHOR_END)?;
            self.select(caret, false)
        }
    }

    /// The caret while composing, as `imeCaret` says.
    fn place_caret(&self, range: &ITfRange) -> Result<()> {
        match self.ime.caret {
            Caret::End => self.caret_after(range),
            Caret::Keep => Ok(()),
            Caret::Start => unsafe {
                let caret = range.Clone()?;
                caret.Collapse(self.ec, TF_ANCHOR_START)?;
                self.select(caret, false)
            },
            Caret::Interim => unsafe {
                let last = range.Clone()?;
                let acp: ITfRangeACP = last.cast()?;
                let (mut start, mut length) = (0i32, 0i32);
                acp.GetExtent(&mut start, &mut length)?;
                if length < 1 {
                    return self.caret_after(range);
                }
                acp.SetExtent(start + length - 1, 1)?;
                self.select(last, true)
            },
        }
    }

    fn set_attribute(&self, range: &ITfRange) -> Result<()> {
        if self.atom == 0 {
            return Ok(());
        }
        unsafe {
            let property = self.context.GetProperty(&GUID_PROP_ATTRIBUTE)?;
            property.SetValue(self.ec, range, &variant_i4(self.atom as i32))
        }
    }

    fn clear_attribute(&self, range: &ITfRange) -> Result<()> {
        unsafe {
            let property = self.context.GetProperty(&GUID_PROP_ATTRIBUTE)?;
            property.Clear(self.ec, range)
        }
    }

    /// A range over `r` (document offsets), through ITfRangeACP.
    fn range_at(&self, r: TextRange) -> Result<ITfRange> {
        unsafe {
            let range = self.context.GetStart(self.ec)?;
            let acp: ITfRangeACP = range.cast()?;
            acp.SetExtent(r.location as i32, r.length as i32)?;
            Ok(range)
        }
    }

    fn extent(range: &ITfRange) -> Option<TextRange> {
        let acp: ITfRangeACP = range.cast().ok()?;
        let (mut start, mut length) = (0i32, 0i32);
        unsafe { acp.GetExtent(&mut start, &mut length).ok()? };
        (start >= 0 && length >= 0).then(|| TextRange::new(start as usize, length as usize))
    }

    fn try_set_marked_text(&mut self, text: &str) -> Result<()> {
        if self.imm.is_some() {
            self.settle()?;
            if self.committing() {
                self.queue(Queued::Show(text.to_string()));
                return Ok(());
            }
            return self.imm_set(text);
        }
        if text.is_empty() {
            if let Some(composition) = self.composition.take() {
                unsafe {
                    let range = composition.GetRange()?;
                    range.SetText(self.ec, 0, &[])?;
                    composition.EndComposition(self.ec)?;
                }
            }
            return Ok(());
        }
        let range = match self.composition.as_ref() {
            Some(composition) => unsafe { composition.GetRange()? },
            None => {
                let at = self.insert_point()?;
                let composition_context: ITfContextComposition = self.context.cast()?;
                let composition = unsafe { composition_context.StartComposition(self.ec, &at, self.sink)? };
                crate::log!("composition started");
                let range = unsafe { composition.GetRange()? };
                *self.composition = Some(composition);
                range
            }
        };
        unsafe { range.SetText(self.ec, 0, &utf16(text))? };
        let range = match self.composition.as_ref() {
            Some(composition) => unsafe { composition.GetRange()? },
            None => range,
        };
        if self.ime.attribute {
            let _ = self.set_attribute(&range);
        }
        self.place_caret(&range)
    }

    fn try_insert_text(&mut self, text: &str, replacing: Option<TextRange>) -> Result<()> {
        if replacing.is_none() && self.imm.is_some() {
            self.settle()?;
            if self.committing() {
                self.queue(Queued::Commit(text.to_string()));
                return Ok(());
            }
            return self.imm_commit(text);
        }
        if let Some(r) = replacing {
            let range = self.range_at(r)?;
            unsafe { range.SetText(self.ec, 0, &utf16(text))? };
            return self.caret_after(&range);
        }
        if let Some(composition) = self.composition.take() {
            unsafe {
                let range = composition.GetRange()?;
                range.SetText(self.ec, 0, &utf16(text))?;
                let range = composition.GetRange()?;
                let _ = self.clear_attribute(&range);
                let _ = self.caret_after(&range);
                composition.EndComposition(self.ec)?;
                crate::log!("committed {} characters, composition ended", text.chars().count());
            }
            return Ok(());
        }
        if text.is_empty() {
            return Ok(());
        }
        let insert: ITfInsertAtSelection = self.context.cast()?;
        let range = unsafe { insert.InsertTextAtSelection(self.ec, Default::default(), &utf16(text))? };
        self.caret_after(&range)
    }
}

// IMM32 apps under Korean rules: one composed character at a time.
impl TsfClient<'_> {
    /// Whether the document is an IMM32 app's (through Windows' bridge).
    pub fn is_imm(&self) -> bool {
        self.imm.is_some()
    }

    fn delivered(&self) -> String {
        self.imm.as_ref().map(|w| w.delivered.clone()).unwrap_or_default()
    }

    /// Removes what is composed without the app getting it.
    fn end_empty(&mut self) -> Result<()> {
        if let Some(composition) = self.composition.take() {
            unsafe {
                let range = composition.GetRange()?;
                range.SetText(self.ec, 0, &[])?;
                composition.EndComposition(self.ec)?;
            }
        }
        Ok(())
    }

    /// Shows the word as `text` when the app already has `delivered` of it.
    fn imm_set(&mut self, text: &str) -> Result<()> {
        let delivered = self.delivered();
        let n = delivered.chars().count();
        let k = common_prefix(&delivered, text);
        if k < n {
            // Characters the app has must change: it erases them first.
            self.end_empty()?;
            let rest: String = text.chars().skip(k).collect();
            let erase = n - k;
            let backspace = self.backspace;
            let word = self.imm.as_mut().expect("imm");
            word.delivered = delivered.chars().take(k).collect();
            if erase == 1 && rest.is_empty() && backspace {
                word.pass_key = true;
            } else {
                word.pending = Some(Pending { erase, text: rest, commit: false });
            }
            crate::log!("imm: {erase} delivered characters to erase");
            return Ok(());
        }
        let rest: String = text.chars().skip(n).collect();
        self.compose_tail(&rest)
    }

    /// Composes `rest` after what the app has: all but its last character go to the app (as the
    /// bridge would hand them over anyway), the last stays composed as an interim character.
    pub fn compose_tail(&mut self, rest: &str) -> Result<()> {
        if rest.is_empty() {
            return self.end_empty();
        }
        // One character per edit (see `defer`): a new composition starts with one character,
        // and each edit hands over one in place of the one composed.
        let all: Vec<char> = rest.chars().collect();
        if self.composition.is_none() && all.len() > 1 {
            self.compose_tail(&all[0].to_string())?;
            self.defer(rest.to_string(), false);
            return Ok(());
        }
        if self.composition.is_some() && all.len() > 1 && self.too_soon() {
            self.defer(rest.to_string(), false);
            return Ok(());
        }
        if all.len() > 2 {
            self.compose_tail(&all[..2].iter().collect::<String>())?;
            self.defer(all[1..].iter().collect(), false);
            return Ok(());
        }
        let range = match self.composition.as_ref() {
            Some(composition) => unsafe { composition.GetRange()? },
            None => {
                let at = self.insert_point()?;
                let composition_context: ITfContextComposition = self.context.cast()?;
                let composition = unsafe { composition_context.StartComposition(self.ec, &at, self.sink)? };
                let range = unsafe { composition.GetRange()? };
                *self.composition = Some(composition);
                self.stepped();
                range
            }
        };
        unsafe { range.SetText(self.ec, 0, &utf16(rest))? };
        let chars: Vec<char> = rest.chars().collect();
        let composition = self.composition.as_ref().expect("composition").clone();
        if chars.len() > 1 {
            let last = chars[chars.len() - 1];
            let range = unsafe { composition.GetRange()? };
            if let Some(extent) = Self::extent(&range) {
                let tail = unsafe { range.Clone()? };
                let acp: ITfRangeACP = tail.cast()?;
                let width = last.len_utf16();
                unsafe {
                    acp.SetExtent((extent.location + extent.length - width) as i32, width as i32)?;
                    composition.ShiftStart(self.ec, &tail)?;
                }
                self.stepped();
                let given: String = chars[..chars.len() - 1].iter().collect();
                if let Some(word) = self.imm.as_mut() {
                    word.delivered.push_str(&given);
                }
            }
        }
        let range = unsafe { composition.GetRange()? };
        if self.ime.attribute {
            let _ = self.set_attribute(&range);
        }
        self.select(unsafe { range.Clone()? }, true)
    }

    /// Ends the word as `text` when the app already has `delivered` of it.
    fn imm_commit(&mut self, text: &str) -> Result<()> {
        let delivered = self.delivered();
        let n = delivered.chars().count();
        let k = common_prefix(&delivered, text);
        if k < n {
            self.end_empty()?;
            let word = self.imm.as_mut().expect("imm");
            word.pending = Some(Pending { erase: n - k, text: text.chars().skip(k).collect(), commit: true });
            word.delivered.clear();
            crate::log!("imm: {} delivered characters to erase before the commit", n - k);
            return Ok(());
        }
        let rest: String = text.chars().skip(n).collect();
        if let Some(word) = self.imm.as_mut() {
            word.delivered.clear();
        }
        self.commit_text(&rest)
    }

    /// Whether the character composed began too recently to be handed over
    /// (`inject::CONTINUE_DELAY`): Tabby (xterm.js) notes where a composition ends only on a
    /// timer after it changes, and one handed over before that timer ran was lost (claude typed
    /// fast came out clade).
    fn too_soon(&self) -> bool {
        self.composition.is_some()
            && self
                .imm
                .as_ref()
                .and_then(|w| w.last_step)
                .is_some_and(|t| t.elapsed() < crate::inject::CONTINUE_DELAY)
    }

    fn stepped(&mut self) {
        if let Some(word) = self.imm.as_mut() {
            word.last_step = Some(std::time::Instant::now());
        }
    }

    /// The rest of a change for the next edit, after the marker key comes back. Under Korean
    /// rules the app gets one character at a time, each in place of the one composed, and some
    /// apps keep only as many characters as were composed (Tabby, through xterm.js: 래 changed
    /// to `for` at once typed f, then r). Edits of one session reach the app as one change, so a
    /// longer one goes in over several.
    fn defer(&mut self, text: String, commit: bool) {
        if let Some(word) = self.imm.as_mut() {
            crate::log!("imm: {} characters to go on after the marker", text.chars().count());
            word.pending = Some(Pending { erase: 0, text, commit });
        }
    }

    /// A step of a longer change left for the marker key, when the next key comes first: the
    /// rest of a commit goes in now (all at once), the rest of what was composed is redone.
    fn settle(&mut self) -> Result<()> {
        let Some(word) = self.imm.as_mut() else { return Ok(()) };
        if matches!(word.pending, Some(Pending { erase: 0, commit: false, .. })) {
            word.pending = None;
        }
        Ok(())
    }

    /// A commit is still going in: what comes now waits behind it.
    fn committing(&self) -> bool {
        self.imm.as_ref().is_some_and(|w| matches!(w.pending, Some(Pending { erase: 0, commit: true, .. })))
    }

    fn queue(&mut self, item: Queued) {
        let Some(word) = self.imm.as_mut() else { return };
        // A commit ends the word that was showing; a new display replaces the last one.
        if matches!(word.queue.last(), Some(Queued::Show(_))) {
            word.queue.pop();
        }
        word.queue.push(item);
    }

    /// Types `text` after the word: behind the rest of a commit still to go in, if any.
    pub fn type_after(&mut self, text: &str) -> Result<()> {
        if self.committing() {
            self.queue(Queued::Commit(text.to_string()));
            return Ok(());
        }
        self.commit_text(text)
    }

    /// Puts `text` in as typed: through a composition ended at once, which the bridge passes on;
    /// in an IMM32 app one character per edit.
    pub fn commit_text(&mut self, text: &str) -> Result<()> {
        if self.imm.is_some() {
            let all: Vec<char> = text.chars().collect();
            if (self.composition.is_some() || !all.is_empty()) && self.too_soon() {
                self.defer(text.to_string(), true);
                return Ok(());
            }
            if all.len() > 1 {
                if self.composition.is_none() {
                    self.compose_tail(&all[0].to_string())?;
                    self.defer(text.to_string(), true);
                } else {
                    self.compose_tail(&all[..2].iter().collect::<String>())?;
                    if let Some(word) = self.imm.as_mut() {
                        word.delivered.clear();
                    }
                    self.defer(all[1..].iter().collect(), true);
                }
                return Ok(());
            }
        }
        self.commit_at_once(text)
    }

    fn commit_at_once(&mut self, text: &str) -> Result<()> {
        if self.composition.is_none() && text.is_empty() {
            return Ok(());
        }
        if self.composition.is_none() {
            let at = self.insert_point()?;
            let composition_context: ITfContextComposition = self.context.cast()?;
            let composition = unsafe { composition_context.StartComposition(self.ec, &at, self.sink)? };
            *self.composition = Some(composition);
        }
        let composition = self.composition.take().expect("composition");
        unsafe {
            let range = composition.GetRange()?;
            range.SetText(self.ec, 0, &utf16(text))?;
            let range = composition.GetRange()?;
            let _ = self.clear_attribute(&range);
            let _ = self.caret_after(&range);
            composition.EndComposition(self.ec)?;
        }
        Ok(())
    }

    /// Finishes a change the app erased characters for: the composed rest, or the word's end.
    pub fn finish_pending(&mut self, pending: Pending) -> Result<()> {
        if pending.commit {
            self.commit_text(&pending.text)?;
        } else {
            self.compose_tail(&pending.text)?;
        }
        // Then what waited behind a commit, as far as it goes now.
        while !self.committing() {
            let Some(word) = self.imm.as_mut() else { break };
            if word.queue.is_empty() {
                break;
            }
            match word.queue.remove(0) {
                Queued::Commit(text) => {
                    word.delivered.clear();
                    self.commit_text(&text)?;
                }
                Queued::Show(text) => self.imm_set(&text)?,
            }
        }
        Ok(())
    }
}

impl TextClient for TsfClient<'_> {
    fn delivers_all_but_last(&self) -> bool {
        self.is_imm()
    }

    fn set_marked_text(&mut self, text: &str) {
        if let Err(e) = self.try_set_marked_text(text) {
            crate::log!("set marked text failed: {e}");
        }
    }

    fn insert_text(&mut self, text: &str, replacing: Option<TextRange>) {
        if let Err(e) = self.try_insert_text(text, replacing) {
            crate::log!("insert text failed: {e}");
        }
    }

    fn selected_range(&self) -> TextRange {
        unsafe {
            let mut selection = [TF_SELECTION::default()];
            let mut fetched = 0u32;
            if self.context.GetSelection(self.ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched).is_err() || fetched == 0 {
                return TextRange::NONE;
            }
            let range = ManuallyDrop::take(&mut selection[0].range);
            range.as_ref().and_then(Self::extent).unwrap_or(TextRange::NONE)
        }
    }

    fn marked_range(&self) -> TextRange {
        self.composition
            .as_ref()
            .and_then(|c| unsafe { c.GetRange().ok() })
            .and_then(|r| Self::extent(&r))
            .unwrap_or(TextRange::NONE)
    }

    fn text_in(&self, r: TextRange) -> Option<String> {
        if !r.is_found() {
            return None;
        }
        let range = self.range_at(r).ok()?;
        let mut buffer = vec![0u16; r.length.max(1)];
        let mut fetched = 0u32;
        unsafe { range.GetText(self.ec, 0, &mut buffer[..r.length], &mut fetched).ok()? };
        String::from_utf16(&buffer[..fetched as usize]).ok()
    }
}
