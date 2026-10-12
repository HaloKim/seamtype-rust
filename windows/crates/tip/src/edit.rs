//! Edit sessions: TSF only lets a text service read or change a document inside one, through
//! the edit cookie it hands to `DoEditSession`.

use std::cell::RefCell;

use windows::Win32::UI::TextServices::{
    ITfContext, ITfEditSession, ITfEditSession_Impl, TF_CONTEXT_EDIT_CONTEXT_FLAGS, TF_ES_READWRITE, TF_ES_SYNC,
};
use windows_core::{implement, Result, HRESULT};

type Callback = Box<dyn FnOnce(u32) -> Result<()>>;

#[implement(ITfEditSession)]
struct EditSession {
    callback: RefCell<Option<Callback>>,
}

impl ITfEditSession_Impl for EditSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        let callback = self.callback.borrow_mut().take();
        match callback {
            Some(f) => crate::guard(|| f(ec)),
            None => Ok(()),
        }
    }
}

/// Asks for an edit session; `Ok(true)` when it ran (synchronously) and succeeded.
pub fn request(context: &ITfContext, client_id: u32, flags: TF_CONTEXT_EDIT_CONTEXT_FLAGS, f: impl FnOnce(u32) -> Result<()> + 'static) -> Result<bool> {
    let session: ITfEditSession = EditSession { callback: RefCell::new(Some(Box::new(f))) }.into();
    let result: HRESULT = unsafe { context.RequestEditSession(client_id, &session, flags)? };
    Ok(result.is_ok() && result.0 == 0)
}

/// A read/write session that runs before this returns, or not at all.
pub fn sync_write(context: &ITfContext, client_id: u32, f: impl FnOnce(u32) -> Result<()> + 'static) -> Result<bool> {
    request(context, client_id, TF_ES_SYNC | TF_ES_READWRITE, f)
}
