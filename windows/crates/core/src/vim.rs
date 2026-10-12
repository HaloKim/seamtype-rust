//! What vim reports through `seamtype --vim <mode>` on each mode change (VimSignal.swift).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VimSignal {
    /// Insert (or replace) mode: words are decided.
    Insert,
    /// Normal, visual or command-line mode: keys go straight to vim.
    Normal,
    /// Vim quit, was suspended or lost focus: whatever is in front now isn't vim.
    Off,
}

impl VimSignal {
    pub fn parse(s: &str) -> Option<VimSignal> {
        match s {
            "insert" => Some(VimSignal::Insert),
            "normal" => Some(VimSignal::Normal),
            "off" => Some(VimSignal::Off),
            _ => None,
        }
    }

    pub fn raw_value(self) -> &'static str {
        match self {
            VimSignal::Insert => "insert",
            VimSignal::Normal => "normal",
            VimSignal::Off => "off",
        }
    }
}
