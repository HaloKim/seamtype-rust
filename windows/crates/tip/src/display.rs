//! The look of a word being composed: underlined, in the text's own colors, as marked text is
//! on macOS. Apps ask the display attribute provider (the text service's COM class) for it.

use std::cell::Cell;

use windows::Win32::Foundation::{E_INVALIDARG, S_FALSE};
use windows::Win32::UI::TextServices::{
    IEnumTfDisplayAttributeInfo, IEnumTfDisplayAttributeInfo_Impl, ITfDisplayAttributeInfo, ITfDisplayAttributeInfo_Impl,
    TF_ATTR_INPUT, TF_CT_NONE, TF_DA_COLOR, TF_DISPLAYATTRIBUTE, TF_LS_SOLID,
};
use windows_core::{implement, Result, BSTR, GUID};

use crate::GUID_DISPLAY_ATTRIBUTE_INPUT;

pub fn input_attribute() -> TF_DISPLAYATTRIBUTE {
    let none = TF_DA_COLOR { r#type: TF_CT_NONE, ..Default::default() };
    TF_DISPLAYATTRIBUTE {
        crText: none,
        crBk: none,
        lsStyle: TF_LS_SOLID,
        fBoldLine: false.into(),
        crLine: none,
        bAttr: TF_ATTR_INPUT,
    }
}

#[implement(ITfDisplayAttributeInfo)]
pub struct InputAttribute;

impl ITfDisplayAttributeInfo_Impl for InputAttribute_Impl {
    fn GetGUID(&self) -> Result<GUID> {
        Ok(GUID_DISPLAY_ATTRIBUTE_INPUT)
    }

    fn GetDescription(&self) -> Result<BSTR> {
        Ok(BSTR::from("Seamtype composition"))
    }

    fn GetAttributeInfo(&self, attribute: *mut TF_DISPLAYATTRIBUTE) -> Result<()> {
        if attribute.is_null() {
            return Err(E_INVALIDARG.into());
        }
        unsafe { *attribute = input_attribute() };
        Ok(())
    }

    fn SetAttributeInfo(&self, _attribute: *const TF_DISPLAYATTRIBUTE) -> Result<()> {
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        Ok(())
    }
}

/// Enumerates the one display attribute.
#[implement(IEnumTfDisplayAttributeInfo)]
pub struct AttributeEnum {
    pub index: Cell<u32>,
}

impl IEnumTfDisplayAttributeInfo_Impl for AttributeEnum_Impl {
    fn Clone(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        Ok(AttributeEnum { index: Cell::new(self.index.get()) }.into())
    }

    fn Next(&self, count: u32, info: *mut Option<ITfDisplayAttributeInfo>, fetched: *mut u32) -> Result<()> {
        let mut n = 0u32;
        if count > 0 && self.index.get() == 0 && !info.is_null() {
            unsafe { *info = Some(InputAttribute.into()) };
            self.index.set(1);
            n = 1;
        }
        if !fetched.is_null() {
            unsafe { *fetched = n };
        }
        if n == count { Ok(()) } else { Err(S_FALSE.into()) }
    }

    fn Reset(&self) -> Result<()> {
        self.index.set(0);
        Ok(())
    }

    fn Skip(&self, count: u32) -> Result<()> {
        let skipped = count.min(1u32.saturating_sub(self.index.get()));
        self.index.set(self.index.get() + skipped);
        if skipped == count { Ok(()) } else { Err(S_FALSE.into()) }
    }
}
