// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! GDI handles that free themselves, and the off-screen surface everything is drawn onto.
//!
//! **Windows only.** There is nothing portable in here to test — which is the point of keeping
//! it in one file: `doc/windows-shell.md` names "a leaked `HFONT` per keystroke" as the classic
//! version of this shell's `unsafe` risk, and the mitigation is that no other file creates a
//! GDI object at all.
//!
//! A GDI object is a process-wide resource with a hard limit (10 000 per process by default),
//! and `DeleteObject` on a handle that is still *selected into a DC* silently does nothing —
//! which is how a leak that looks like it was freed happens. Both types below therefore own
//! their handle for a scope that is strictly inside the DC's, and [`Selected`] puts the
//! previous object back before anything is deleted.

#![cfg(windows)]

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW,
    CreatePen, CreateSolidBrush, DIB_RGB_COLORS, DeleteDC, DeleteObject, FW_BOLD, FW_NORMAL,
    FillRect, GdiFlush, GetTextExtentPoint32W, HBITMAP, HBRUSH, HDC, HFONT, HGDIOBJ, HPEN,
    LOGFONTW, PS_SOLID, RoundRect, SRCCOPY, SelectObject,
};

use crate::theme::Rgb;

/// A brush that deletes itself.
pub struct Brush(HBRUSH);

impl Brush {
    pub fn solid(colour: Rgb) -> Self {
        // SAFETY: creating a brush touches nothing but the GDI handle table.
        Self(unsafe { CreateSolidBrush(windows::Win32::Foundation::COLORREF(colour.colorref())) })
    }

    pub fn handle(&self) -> HBRUSH {
        self.0
    }
}

impl Drop for Brush {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and was never selected into a DC — `FillRect` takes a
        // brush as an argument rather than selecting it, which is why filling is done that way
        // throughout this shell.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}

/// A pen that deletes itself.
///
/// One width and one colour, which is every line this shell draws that is not a filled
/// rectangle — in practice the outline of a [`round_rect`], since everything else is a `fill`.
pub struct Pen(HPEN);

impl Pen {
    pub fn solid(colour: Rgb, width: i32) -> Self {
        // SAFETY: creating a pen touches nothing but the GDI handle table.
        Self(unsafe {
            CreatePen(
                PS_SOLID,
                width.max(1),
                windows::Win32::Foundation::COLORREF(colour.colorref()),
            )
        })
    }
}

impl Drop for Pen {
    fn drop(&mut self) {
        // SAFETY: the handle is ours, and `Selected` puts the DC's previous pen back before this
        // runs — a selected object is one `DeleteObject` quietly declines to free.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}

/// A font that deletes itself.
///
/// Built from a face name, a height in pixels and a weight rather than from a `TextStyle`,
/// because the mapping from one to the other is `metrics.rs`' job and this file is meant to stay
/// the only place a handle is created rather than becoming the place decisions are made.
pub struct Font(HFONT);

impl Font {
    /// `height` is a *cell* height in pixels — the negative `lfHeight` convention, which asks
    /// GDI for a font whose character height is that rather than whose line box is.
    pub fn new(face: &str, height: i32, bold: bool) -> Self {
        Self::styled(face, height, bold, false, false, false)
    }

    /// The same, with the three other `LOGFONTW` switches a run of text can ask for.
    ///
    /// Italic, underline and strikethrough are properties of the *font* in GDI rather than of a
    /// separate drawing call, which is why the text pane gets all three for nothing where the GTK
    /// window needed Pango attributes for the last two (W5, `metrics.rs`).
    pub fn styled(
        face: &str,
        height: i32,
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
    ) -> Self {
        let mut log = LOGFONTW {
            lfHeight: -height,
            lfWeight: if bold {
                FW_BOLD.0 as i32
            } else {
                FW_NORMAL.0 as i32
            },
            lfItalic: u8::from(italic),
            lfUnderline: u8::from(underline),
            lfStrikeOut: u8::from(strike),
            ..Default::default()
        };
        for (slot, unit) in log.lfFaceName.iter_mut().zip(face.encode_utf16()) {
            *slot = unit;
        }
        // A `LOGFONTW` face name is 32 units *including* the terminator, and a longer name is
        // simply truncated by the loop above — which would leave no NUL. Zeroed by
        // `Default::default()` above, so the last slot is only ever written when the name is 31
        // units or fewer; assert that rather than trust it.
        log.lfFaceName[31] = 0;
        // SAFETY: `log` is a fully initialised local read only for the duration of the call.
        Self(unsafe { CreateFontIndirectW(&log) })
    }

    pub fn handle(&self) -> HFONT {
        self.0
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: the handle is ours, and `Selected` guarantees it is not selected into any DC
        // by the time this runs.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}

/// An object selected into a DC for a scope, putting the previous one back on the way out.
///
/// This is the half that actually prevents the leak: `DeleteObject` on a selected handle fails
/// quietly, so a font has to be *deselected* before it is dropped, and the only reliable way to
/// pair those is a guard.
pub struct Selected<'a> {
    dc: HDC,
    previous: HGDIOBJ,
    _keep: std::marker::PhantomData<&'a ()>,
}

impl<'a> Selected<'a> {
    pub fn font(dc: HDC, font: &'a Font) -> Self {
        Self::object(dc, HGDIOBJ(font.handle().0))
    }

    pub fn pen(dc: HDC, pen: &'a Pen) -> Self {
        Self::object(dc, HGDIOBJ(pen.0.0))
    }

    pub fn brush(dc: HDC, brush: &'a Brush) -> Self {
        Self::object(dc, HGDIOBJ(brush.0.0))
    }

    fn object(dc: HDC, object: HGDIOBJ) -> Self {
        // SAFETY: `dc` is live for the caller's scope and the object outlives this guard.
        let previous = unsafe { SelectObject(dc, object) };
        Self {
            dc,
            previous,
            _keep: std::marker::PhantomData,
        }
    }
}

impl Drop for Selected<'_> {
    fn drop(&mut self) {
        // SAFETY: restoring the object the DC had before this guard was made.
        unsafe {
            SelectObject(self.dc, self.previous);
        }
    }
}

/// An off-screen surface the size of the client area, blitted over the window in one move.
///
/// This is the whole of the flicker answer, and it is why `WM_ERASEBKGND` is answered with 1
/// rather than left to `DefWindowProc`: the default erases the client area with the class brush
/// *before* `WM_PAINT` runs, so a window that then draws its own background flashes it. Nothing
/// is erased, every pixel is written here, and the blit replaces the lot at once.
pub struct BackBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
}

impl BackBuffer {
    /// A surface compatible with `target`, of the given size.
    ///
    /// `None` when GDI declines — which happens when the window has been sized to nothing, and
    /// the caller's response is to draw nothing at all rather than to fail.
    pub fn new(target: HDC, width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        // SAFETY: `target` is the DC the caller is painting with; both objects are released in
        // `Drop`, in the reverse order.
        unsafe {
            let dc = CreateCompatibleDC(Some(target));
            if dc.is_invalid() {
                return None;
            }
            let bitmap = CreateCompatibleBitmap(target, width, height);
            if bitmap.is_invalid() {
                let _ = DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
            Some(Self {
                dc,
                bitmap,
                previous,
                width,
                height,
            })
        }
    }

    pub fn dc(&self) -> HDC {
        self.dc
    }

    /// Paint the whole surface one colour — the first thing every frame does, so that no pixel
    /// carries over from the last one.
    pub fn clear(&self, colour: Rgb) {
        let brush = Brush::solid(colour);
        let rect = RECT {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height,
        };
        // SAFETY: the DC and the brush are both live, and the rectangle is the surface's own.
        unsafe {
            FillRect(self.dc, &rect, brush.handle());
        }
    }

    /// Put the finished frame on screen.
    pub fn present(&self, target: HDC) {
        // SAFETY: both DCs are live and the rectangle is within both surfaces.
        unsafe {
            let _ = BitBlt(
                target,
                0,
                0,
                self.width,
                self.height,
                Some(self.dc),
                0,
                0,
                SRCCOPY,
            );
        }
    }
}

impl Drop for BackBuffer {
    fn drop(&mut self) {
        // SAFETY: the bitmap is deselected before it is deleted, and the DC is deleted last.
        // Doing this the other way round is the leak this file exists to make impossible.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Fill one rectangle. A free function rather than a method so that both the window path and
/// (from W2) the DIB render path reach it the same way.
pub fn fill(dc: HDC, left: i32, top: i32, right: i32, bottom: i32, colour: Rgb) {
    if right <= left || bottom <= top {
        return;
    }
    let brush = Brush::solid(colour);
    let rect = RECT {
        left,
        top,
        right,
        bottom,
    };
    // SAFETY: the DC is the caller's and live; the brush outlives the call.
    unsafe {
        FillRect(dc, &rect, brush.handle());
    }
}

/// One rectangle with rounded corners, filled and outlined.
///
/// The whole of this shell's softening: the name box, the formula bar and the assist band's chip
/// are drawn with one, everything else is still a square `fill`. GDI has no antialiasing, so the
/// corners are cut rather than smoothed — at the two- to four-pixel radius used here that reads
/// as a rounded field and not as a staircase, and it costs one call rather than a bitmap.
///
/// A radius of zero, or a rectangle too small for the one asked for, comes out square: a corner
/// bigger than the box it is cutting is how a field turns into an ellipse.
pub fn round_rect(dc: HDC, rect: RECT, radius: i32, fill: Rgb, border: Rgb) {
    let RECT {
        left,
        top,
        right,
        bottom,
    } = rect;
    if right <= left || bottom <= top {
        return;
    }
    let radius = radius
        .max(0)
        .min((right - left) / 2)
        .min((bottom - top) / 2);
    let brush = Brush::solid(fill);
    let pen = Pen::solid(border, 1);
    let _brush = Selected::brush(dc, &brush);
    let _pen = Selected::pen(dc, &pen);
    // SAFETY: the DC is the caller's and live, and both objects are selected for this scope and
    // put back by the guards before either is dropped.
    unsafe {
        let _ = RoundRect(dc, left, top, right, bottom, radius * 2, radius * 2);
    }
}

/// How wide a string is in the DC's current font, in pixels.
///
/// What the drawing code needs to place one run of text after another — the assist band's
/// signature, whose emphasised argument is a separate `TextOut` in a bolder font, and the status
/// bar's two halves. Measuring and drawing therefore use one font and one engine, which is the
/// same rule `metrics.rs` follows for the text pane and for the same reason.
pub fn text_width(dc: HDC, text: &str) -> i32 {
    let wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() {
        return 0;
    }
    let mut size = windows::Win32::Foundation::SIZE::default();
    // SAFETY: the buffer and the size are live locals that outlive the call.
    unsafe {
        let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
    }
    size.cx
}

/// The client area, as GDI measures it.
pub fn client_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    // SAFETY: `hwnd` is this window's and `rect` is a live local.
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
    }
    rect
}

/// A Rust string as the NUL-terminated UTF-16 every `…W` entry point wants.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A drawing surface with **no window, no compositor and no display** — the whole of
/// `--render-to` (`doc/windows-shell.md`, decision 5).
///
/// `CreateCompatibleDC(None)` gives a memory DC that is not derived from any window, and
/// `CreateDIBSection` gives it pixels this process can read back afterwards. That combination is
/// why one frame can be drawn and written out on a headless `windows-latest` runner, and under
/// Wine on a Linux one: nothing here asks the window manager for anything.
///
/// **24 bits per pixel, bottom-up**, and both halves are for the same reason — the bits a
/// section hands back are then already in a `.bmp` file's own layout, so [`Dib::bmp`] is a
/// 54-byte header in front of them rather than an encoder. A 32-bit section would have a fourth
/// byte per pixel that GDI leaves undefined, which is exactly the thing a byte-for-byte
/// comparison must not depend on.
pub struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
}

impl Dib {
    /// A BMP row is padded to a four-byte boundary, which is a property of the *format* rather
    /// than of this code — `CreateDIBSection` lays its rows out the same way.
    fn stride(width: i32) -> usize {
        (width as usize * 3).div_ceil(4) * 4
    }

    pub fn new(width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).expect("forty"),
                biWidth: width,
                // Positive: a bottom-up DIB, which is what a `.bmp` file holds. GDI still draws
                // with y increasing downwards — the orientation is the memory layout's, not the
                // coordinate system's, so nothing above this has to know.
                biHeight: height,
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `info` is a fully initialised local read for the length of the call, and
        // `bits` is written by it with a pointer owned by the bitmap — released in `Drop`.
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
                .ok()
                .filter(|bitmap| !bitmap.is_invalid());
            let Some(bitmap) = bitmap else {
                let _ = DeleteDC(dc);
                return None;
            };
            let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
            Some(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
            })
        }
    }

    pub fn dc(&self) -> HDC {
        self.dc
    }

    /// The surface as the bytes of a `.bmp` file.
    ///
    /// No encoder and no dependency: a 14-byte file header, the 40-byte info header the section
    /// was made with, and the bits themselves, which are already in the right order and the
    /// right padding. That is also what makes the output *comparable* — two runs that drew the
    /// same thing produce the same bytes, with nothing in the file that could differ.
    pub fn bmp(&self) -> Vec<u8> {
        // GDI batches drawing calls; without this the bits may not have been written yet, and
        // the frame would be missing whatever was still in the queue.
        // SAFETY: no arguments.
        unsafe {
            let _ = GdiFlush();
        }
        let stride = Self::stride(self.width);
        let pixels = stride * self.height as usize;
        let header = 14 + std::mem::size_of::<BITMAPINFOHEADER>();
        let mut out = Vec::with_capacity(header + pixels);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(
            &u32::try_from(header + pixels)
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&u32::try_from(header).expect("small").to_le_bytes());
        out.extend_from_slice(&u32::try_from(header - 14).expect("forty").to_le_bytes());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        out.extend_from_slice(&u32::try_from(pixels).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&[0u8; 16]); // resolution, palette counts: all zero, all optional
        // SAFETY: the section owns `pixels` bytes at `self.bits` until this object is dropped,
        // and nothing else is writing them — this process is the only drawer and `GdiFlush`
        // above has finished the batch.
        out.extend_from_slice(unsafe { std::slice::from_raw_parts(self.bits, pixels) });
        out
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: the bitmap is deselected before it is deleted and the DC deleted last — the
        // same order, and for the same reason, as `BackBuffer`.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Composite a decoded picture onto `dc` — `image.rs`'s only caller, and the reason this is here
/// rather than there: a source `HBITMAP` is a GDI object, and this file is the only one that
/// creates one.
///
/// `pixels` is `src_w * src_h * 4` bytes of **premultiplied** BGRA, top-down — WIC's
/// `GUID_WICPixelFormat32bppPBGRA` is exactly that layout, which is why `image.rs` asks WIC for
/// it rather than converting afterwards. `AlphaBlend` resamples when `w, h` differ from
/// `src_w, src_h`, which is what lets a picture be drawn at less than its own pixels without a
/// resampler this crate would then own.
pub fn blit_image(
    dc: HDC,
    dest: crate::sheet::geom::Rect,
    size: (u32, u32),
    pixels: &[u8],
) -> bool {
    let (x, y, right, bottom) = dest.edges();
    let (w, h) = (right - x, bottom - y);
    let (src_w, src_h) = size;
    let (Ok(src_w_i), Ok(src_h_i)) = (i32::try_from(src_w), i32::try_from(src_h)) else {
        return false;
    };
    if w <= 0 || h <= 0 || src_w_i <= 0 || src_h_i <= 0 {
        return false;
    }
    let Some(len) = (src_w as usize)
        .checked_mul(src_h as usize)
        .and_then(|n| n.checked_mul(4))
    else {
        return false;
    };
    if pixels.len() < len {
        return false;
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).expect("forty"),
            biWidth: src_w_i,
            // Negative: top-down, matching WIC's own row order — [`Dib`] above is bottom-up
            // instead, for a `.bmp` file's own layout, so nothing about that shape is shared here.
            biHeight: -src_h_i,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `info` is a fully initialised local for the length of the call; `bits` is written
    // by it with a pointer owned by the bitmap and freed by `DeleteObject` before returning.
    unsafe {
        let mem_dc = CreateCompatibleDC(Some(dc));
        if mem_dc.is_invalid() {
            return false;
        }
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(mem_dc);
            return false;
        };
        if bitmap.is_invalid() || bits.is_null() {
            let _ = DeleteDC(mem_dc);
            return false;
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast(), len);
        let previous = SelectObject(mem_dc, HGDIOBJ(bitmap.0));
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let ok = AlphaBlend(dc, x, y, w, h, mem_dc, 0, 0, src_w_i, src_h_i, blend).as_bool();
        SelectObject(mem_dc, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        ok
    }
}
