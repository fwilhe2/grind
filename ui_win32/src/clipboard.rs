// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The system clipboard, `CF_UNICODETEXT` only — and the only file that touches it, so "always
//! closed, even on the early return" is a property of one file. The codec between a rectangle and
//! this module's `String` is `grind_sheet::clip`'s. `CF_UNICODETEXT` is what every other
//! spreadsheet reads (decision 6), so a copy here pastes into LibreOffice Calc or Excel and back.

#![cfg(windows)]

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::core::PCWSTR;

/// Put `text` on the clipboard as `CF_UNICODETEXT`, replacing what was there; `false` when another
/// application had it open or the allocation failed, and then nothing changed. `hwnd` is the owner
/// Win32 asks for, so another application knows whom to ask to keep rendering the data — which
/// this shell does not do.
pub fn set_text(hwnd: HWND, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * std::mem::size_of::<u16>();
    // SAFETY: one `OpenClipboard`, closed on every path below. The block's `?`-shaped chain
    // stops at the first failure and `CloseClipboard` still runs, which is what keeps a half
    // finished copy from leaving the clipboard open for the rest of the process.
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let ok = (|| -> Option<()> {
            EmptyClipboard().ok()?;
            // Moveable, which is what lets ownership of the memory pass to the system on
            // success instead of this function having to free it.
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes).ok()?;
            let dest = GlobalLock(handle);
            if dest.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dest.cast(), wide.len());
            let _ = GlobalUnlock(handle);
            SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(handle.0))).ok()?;
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        ok
    }
}

/// Read the clipboard's `CF_UNICODETEXT`, or `None` — another application's clipboard holds an
/// image, holds nothing, or the clipboard could not be opened at all.
pub fn get_text(hwnd: HWND) -> Option<String> {
    // SAFETY: one `OpenClipboard`, closed on every path. The handle `GetClipboardData` returns
    // is owned by the clipboard, not by this call, so it is read and left alone rather than
    // freed.
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return None;
        }
        let text = (|| -> Option<String> {
            let handle = GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok()?;
            let ptr = GlobalLock(HGLOBAL(handle.0));
            if ptr.is_null() {
                return None;
            }
            let text = PCWSTR(ptr.cast()).to_string().ok();
            let _ = GlobalUnlock(HGLOBAL(handle.0));
            text
        })();
        let _ = CloseClipboard();
        text
    }
}
