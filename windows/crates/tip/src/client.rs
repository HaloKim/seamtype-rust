//! `TextClient` over a TSF document, valid for one edit session (IMKClient in InputController.swift).
//!
//! Marked text is a TSF composition: started at the selection, its text replaced as the word
//! grows, underlined through the display attribute property, and ended when the word is
//! committed. Positions are ACP (UTF-16) offsets, read and set through ITfRangeACP; a text store
//! without them reports no selection, so Esc after a commit simply goes to the app there.

use std::mem::ManuallyDrop;

use seamtype_core::{TextClient, TextRange};
use windows::Win32::System::Variant::{VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfContext, ITfContextComposition, ITfInsertAtSelection, ITfRange, ITfRangeACP,
    GUID_PROP_ATTRIBUTE, TF_AE_NONE, TF_ANCHOR_END, TF_DEFAULT_SELECTION, TF_IAS_QUERYONLY, TF_SELECTION,
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

    /// Puts the caret (an empty selection) at the end of `range`.
    fn caret_after(&self, range: &ITfRange) -> Result<()> {
        unsafe {
            let caret = range.Clone()?;
            caret.Collapse(self.ec, TF_ANCHOR_END)?;
            let selection = TF_SELECTION {
                range: ManuallyDrop::new(Some(caret)),
                style: TF_SELECTIONSTYLE { ase: TF_AE_NONE, fInterimChar: false.into() },
            };
            let result = self.context.SetSelection(self.ec, std::slice::from_ref(&selection));
            drop(ManuallyDrop::into_inner(selection.range));
            result
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
        let _ = self.set_attribute(&range);
        self.caret_after(&range)
    }

    fn try_insert_text(&mut self, text: &str, replacing: Option<TextRange>) -> Result<()> {
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

impl TextClient for TsfClient<'_> {
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
