// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! GDI handles that free themselves, and the off-screen surface everything is drawn onto —
//! **Windows only**, and the one file that creates a GDI object, which is the answer to "a leaked
//! `HFONT` per keystroke". A GDI object is a process-wide resource with a hard limit (10 000 by
//! default), and `DeleteObject` on a handle still *selected into a DC* silently does nothing; so
//! both types here own their handle for a scope strictly inside the DC's, and [`Selected`] puts
//! the previous object back before anything is deleted.

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

/// A pen that deletes itself: one width and colour — in practice a [`round_rect`]'s outline, since
/// everything else is a `fill`.
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

/// A font that deletes itself, built from a face name, a pixel height and a weight; mapping a
/// `TextStyle` onto those is `metrics.rs`' job.
pub struct Font(HFONT);

impl Font {
    /// `height` is a *cell* height in pixels — the negative `lfHeight` convention, which asks
    /// GDI for a font whose character height is that rather than whose line box is.
    pub fn new(face: &str, height: i32, bold: bool) -> Self {
        Self::styled(face, height, bold, false, false, false)
    }

    /// The same, with italic, underline and strikethrough, which GDI makes properties of the font
    /// rather than separate drawing calls.
    pub fn styled(
        face: &str,
        height: i32,
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
    ) -> Self {
        let weight = match bold {
            true => FW_BOLD.0 as i32,
            false => FW_NORMAL.0 as i32,
        };
        Self::with_weight(face, height, weight, italic, underline, strike)
    }

    /// A font at one of the type ramp's weights rather than only regular or bold — *Semibold*
    /// (600) is what Fluent sets a dialog's title and a strong label in, and asking GDI for 700
    /// instead is the difference between a title and a shout.
    pub fn weighted(face: &str, height: i32, weight: i32) -> Self {
        Self::with_weight(face, height, weight, false, false, false)
    }

    fn with_weight(
        face: &str,
        height: i32,
        weight: i32,
        italic: bool,
        underline: bool,
        strike: bool,
    ) -> Self {
        let mut log = LOGFONTW {
            lfHeight: -height,
            lfWeight: weight,
            lfItalic: u8::from(italic),
            lfUnderline: u8::from(underline),
            lfStrikeOut: u8::from(strike),
            ..Default::default()
        };
        for (slot, unit) in log.lfFaceName.iter_mut().zip(face.encode_utf16()) {
            *slot = unit;
        }
        // A `LOGFONTW` face name is 32 units including the terminator, and a longer name is
        // truncated by the loop above, leaving no NUL; the last slot is written only for a name of
        // 31 units or fewer, so this asserts it.
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

/// An object selected into a DC for a scope, putting the previous one back on the way out — what
/// prevents the leak, since `DeleteObject` on a selected handle fails quietly.
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

/// An off-screen surface the size of the client area, blitted over the window in one move — the
/// flicker answer, and why `WM_ERASEBKGND` returns 1: the default erases with the class brush
/// before `WM_PAINT`, and a window drawing its own background would flash it.
pub struct BackBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
}

impl BackBuffer {
    /// A surface compatible with `target`, of the given size. `None` when GDI declines — a window
    /// sized to nothing — and the caller then draws nothing.
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

/// Fill one rectangle — a free function, so the window path and the DIB render path reach it
/// alike.
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

/// One rectangle with rounded corners, filled and outlined — the name box, the formula bar and the
/// assist band's chip; everything else is a square `fill`. GDI has no antialiasing, so the corners
/// are cut, which at two to four pixels reads as rounded. A zero radius, or a rectangle too small
/// for it, comes out square, or a field would turn into an ellipse.
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

/// A small filled triangle pointing down, centred on `(x, y)`, `size` pixels across — a picker's
/// chevron, **drawn rather than typed**: `▾` is not in every font GDI might substitute, and a
/// missing-glyph box says less than no chevron. Rows of one-pixel fills rather than `Polygon`,
/// which needs a pen and a brush and aliases anyway.
pub fn triangle_down(dc: HDC, x: i32, y: i32, size: i32, colour: Rgb) {
    let half = (size / 2).max(1);
    let height = half; // a right-angled pair of slopes: as tall as it is half-wide
    for step in 0..height {
        fill(
            dc,
            x - half + step,
            y + step,
            x + half - step,
            y + step + 1,
            colour,
        );
    }
}

/// How wide a string is in the DC's current font, in pixels — for placing one run after another
/// (the assist band's signature, the status bar's halves) with the font and engine that draw them.
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

/// How tall a line of the DC's current font is, in pixels — what a string as high as its font
/// measures, so a mark under text can be placed where `DrawTextW` put that text.
pub fn line_height(dc: HDC) -> i32 {
    let wide: Vec<u16> = "Ag".encode_utf16().collect();
    let mut size = windows::Win32::Foundation::SIZE::default();
    // SAFETY: the buffer and the size are live locals that outlive the call.
    unsafe {
        let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
    }
    size.cy
}

/// A wavy line from `left` to `right`, crests at `top`: steps of `step` pixels, alternately up and
/// down, each a filled square — `FillRect` lands on the same pixels at every DPI, where a pen's
/// diagonal does not. A misspelt word's mark, in both panes.
pub fn squiggle(dc: HDC, left: f64, right: f64, top: f64, step: f64, ink: Rgb) {
    let step = step.max(1.0).round() as i32;
    let (left, right, top) = (
        left.round() as i32,
        right.round() as i32,
        top.round() as i32,
    );
    let mut x = left;
    let mut down = false;
    while x < right {
        let y = top + if down { step } else { 0 };
        fill(dc, x, y, (x + step).min(right), y + step, ink);
        x += step;
        down = !down;
    }
}

/// A dotted line from `left` to `right` with its dots' tops on `top`: a quieter mark than
/// [`squiggle`], for a grammar finding of typography or style.
pub fn dotted(dc: HDC, left: f64, right: f64, top: f64, dot: f64, ink: Rgb) {
    let dot = dot.max(1.0).round() as i32;
    let (left, right, top) = (
        left.round() as i32,
        right.round() as i32,
        top.round() as i32,
    );
    let mut x = left;
    while x + dot <= right {
        fill(dc, x, top, x + dot, top + dot, ink);
        x += dot * 2;
    }
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

/// The shell font this machine has, asked once. Windows 11's UI face is *Segoe UI Variable*, whose
/// optical sizes GDI sees as three families; `Text` is drawn between 12 and 24 pixels, every size
/// in this shell's ramp. Windows 10 has none. A **probe**, because GDI does not fail on a face it
/// lacks — `CreateFontIndirectW` substitutes silently — and `GetTextFaceW` returns the name asked
/// for; `EnumFontFamiliesExW`'s callback runs only for installed faces.
pub fn ui_face() -> &'static str {
    static FACE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    FACE.get_or_init(|| {
        ["Segoe UI Variable Text", "Segoe UI"]
            .into_iter()
            .find(|face| has_face(face))
            .unwrap_or("Segoe UI")
    })
}

/// The face for text at 20 pixels and over — a dialog's title: *Segoe UI Variable Display*, the
/// optical size Windows 11 draws titles in. The same probe.
pub fn display_face() -> &'static str {
    static FACE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    FACE.get_or_init(|| {
        ["Segoe UI Variable Display", "Segoe UI"]
            .into_iter()
            .find(|face| has_face(face))
            .unwrap_or("Segoe UI")
    })
}

/// Where a line of text goes across the rectangle it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Across {
    Left,
    Centre,
    Right,
}

/// One line of text in the DC's current font, centred on `rect` vertically and placed across it
/// by `across` — cut with an ellipsis rather than clipped mid-glyph when it does not fit, which is
/// what a dialog's labels and a list's rows need and the grid's cells deliberately do not.
pub fn line(dc: HDC, text: &str, rect: RECT, across: Across, ink: Rgb) {
    use windows::Win32::Graphics::Gdi::{
        DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER,
        DrawTextW, SetBkMode, SetTextColor, TRANSPARENT,
    };
    // A slice with no terminator: `DrawTextW` takes a length, and an empty slice is a return
    // rather than a call — `sheet::draw::draw_text` has the history of both.
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() || rect.right <= rect.left {
        return;
    }
    let mut rect = rect;
    let flags = DT_SINGLELINE
        | DT_VCENTER
        | DT_NOPREFIX
        | DT_END_ELLIPSIS
        | match across {
            Across::Left => DT_LEFT,
            Across::Centre => DT_CENTER,
            Across::Right => DT_RIGHT,
        };
    // SAFETY: the DC is the caller's and live; the buffer and rectangle are locals.
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(ink.colorref()));
        DrawTextW(dc, &mut wide, &mut rect, flags);
    }
}

/// One line in the DC's current font turned `degrees` anticlockwise about the middle of `cell`
/// (left, top, right, bottom), and clipped to it.
pub fn turned(dc: HDC, text: &str, cell: (i32, i32, i32, i32), degrees: f64, ink: Rgb) {
    use windows::Win32::Graphics::Gdi::{
        ETO_CLIPPED, ExtTextOutW, GetCurrentObject, GetObjectW, OBJ_FONT, SetBkMode, SetTextAlign,
        SetTextColor, TA_LEFT, TA_TOP, TRANSPARENT,
    };
    let wide: Vec<u16> = crate::sheet::draw::one_line(text).encode_utf16().collect();
    let (left, top, right, bottom) = cell;
    if wide.is_empty() || right <= left || bottom <= top {
        return;
    }
    let mut size = windows::Win32::Foundation::SIZE::default();
    let mut log = LOGFONTW::default();
    // SAFETY: the DC is the caller's and live; every buffer is a local that outlives its call, and
    // the turned font is selected out again before it is deleted.
    unsafe {
        let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
        let current = GetCurrentObject(dc, OBJ_FONT);
        GetObjectW(
            current,
            std::mem::size_of::<LOGFONTW>() as i32,
            Some((&mut log as *mut LOGFONTW).cast()),
        );
        let tenths = (degrees * 10.0).round() as i32;
        log.lfEscapement = tenths;
        log.lfOrientation = tenths;
        let font = CreateFontIndirectW(&log);
        let previous = SelectObject(dc, HGDIOBJ(font.0));
        let middle = (f64::from(left + right) / 2.0, f64::from(top + bottom) / 2.0);
        let (x, y) = crate::sheet::draw::turned_origin(
            f64::from(size.cx),
            f64::from(size.cy),
            middle,
            degrees,
        );
        let clip = RECT {
            left,
            top,
            right,
            bottom,
        };
        let align = SetTextAlign(dc, TA_LEFT | TA_TOP);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(ink.colorref()));
        let _ = ExtTextOutW(
            dc,
            x.round() as i32,
            y.round() as i32,
            ETO_CLIPPED,
            Some(&clip),
            windows::core::PCWSTR(wide.as_ptr()),
            wide.len() as u32,
            None,
        );
        SetTextAlign(dc, windows::Win32::Graphics::Gdi::TEXT_ALIGN_OPTIONS(align));
        SelectObject(dc, previous);
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
}

/// A paragraph in the DC's current font, broken into lines at word boundaries to fit `rect`'s
/// width, from its top. With `measure`, nothing is drawn and the height it *would* take is
/// returned instead — the two share one call so they cannot break the text differently.
pub fn paragraph(dc: HDC, text: &str, rect: RECT, ink: Rgb, measure: bool) -> i32 {
    use windows::Win32::Graphics::Gdi::{
        DT_CALCRECT, DT_LEFT, DT_NOPREFIX, DT_WORDBREAK, DrawTextW, SetBkMode, SetTextColor,
        TRANSPARENT,
    };
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() || rect.right <= rect.left {
        return 0;
    }
    let mut rect = rect;
    let mut flags = DT_LEFT | DT_WORDBREAK | DT_NOPREFIX;
    if measure {
        flags |= DT_CALCRECT;
    }
    // SAFETY: as [`line`].
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(ink.colorref()));
        DrawTextW(dc, &mut wide, &mut rect, flags)
    }
}

/// A DC for measuring with no window behind it, a font already selected — what a modal needs
/// before its window exists, since its size comes from its words.
pub struct Measure<'a> {
    dc: HDC,
    previous: HGDIOBJ,
    _font: std::marker::PhantomData<&'a Font>,
}

impl<'a> Measure<'a> {
    pub fn new(font: &'a Font) -> Option<Self> {
        // SAFETY: a memory DC compatible with the screen, deleted in `Drop` once the font it was
        // given has been deselected again.
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let previous = SelectObject(dc, HGDIOBJ(font.handle().0));
            Some(Self {
                dc,
                previous,
                _font: std::marker::PhantomData,
            })
        }
    }

    pub fn width(&self, text: &str) -> i32 {
        text_width(self.dc, text)
    }

    pub fn height(&self, text: &str, width: i32) -> i32 {
        paragraph(
            self.dc,
            text,
            RECT {
                left: 0,
                top: 0,
                right: width.max(1),
                bottom: 0,
            },
            Rgb(0, 0, 0),
            true,
        )
    }
}

impl Drop for Measure<'_> {
    fn drop(&mut self) {
        // SAFETY: the font is put back before the DC goes — a font left selected in a deleted DC
        // is the leak `Selected` exists to prevent.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteDC(self.dc);
        }
    }
}

/// A check mark — two strokes — inside `rect`, in `colour` at `width` pixels. Drawn rather than
/// typed, for the reason [`triangle_down`] gives.
pub fn check_mark(dc: HDC, rect: RECT, colour: Rgb, width: i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::Polyline;
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    let at = |fx: f64, fy: f64| POINT {
        x: rect.left + (f64::from(w) * fx).round() as i32,
        y: rect.top + (f64::from(h) * fy).round() as i32,
    };
    let pen = Pen::solid(colour, width.max(1));
    let _pen = Selected::pen(dc, &pen);
    // SAFETY: the DC is the caller's; the pen is selected for this scope.
    unsafe {
        let _ = Polyline(dc, &[at(0.2, 0.52), at(0.42, 0.74), at(0.8, 0.3)]);
    }
}

/// Whether GDI has a font family by this name.
fn has_face(name: &str) -> bool {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::Graphics::Gdi::{DEFAULT_CHARSET, EnumFontFamiliesExW, TEXTMETRICW};

    unsafe extern "system" fn found(
        _: *const windows::Win32::Graphics::Gdi::LOGFONTW,
        _: *const TEXTMETRICW,
        _: u32,
        found: LPARAM,
    ) -> i32 {
        // SAFETY: `found` is the `&mut bool` the caller below passed, alive for the whole
        // enumeration — the callback cannot outlive `EnumFontFamiliesExW`.
        unsafe {
            *(found.0 as *mut bool) = true;
        }
        0 // stop at the first match: the question is whether there is one, not how many
    }

    let mut log = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    for (slot, unit) in log.lfFaceName.iter_mut().zip(name.encode_utf16()) {
        *slot = unit;
    }
    log.lfFaceName[31] = 0;
    let mut answer = false;
    // SAFETY: the memory DC is created and deleted here, `log` and `answer` are live locals that
    // outlive the enumeration, and the callback writes only through the pointer it is handed.
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return false;
        }
        EnumFontFamiliesExW(
            dc,
            &log,
            Some(found),
            LPARAM(std::ptr::from_mut(&mut answer) as isize),
            0,
        );
        let _ = DeleteDC(dc);
    }
    answer
}

/// A drawing surface with **no window, no compositor and no display** — `--render-to` (decision
/// 5). `CreateCompatibleDC(None)` gives a memory DC derived from no window and `CreateDIBSection`
/// pixels this process can read back, so a frame draws on a headless `windows-latest` runner or
/// under Wine on Linux.
///
/// **24 bits per pixel, bottom-up**, so the section's bits are already a `.bmp`'s layout and
/// [`Dib::bmp`] is a 54-byte header in front of them. A 32-bit section has a fourth byte GDI
/// leaves undefined, which a byte-for-byte comparison must not depend on.
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

    /// The surface as a `.bmp` file: a 14-byte file header, the 40-byte info header the section
    /// was made with, and the bits, already in order and padded. Nothing in it can differ between
    /// two runs that drew the same thing.
    pub fn bmp(&self) -> Vec<u8> {
        // GDI batches drawing calls; without this the bits may not have been written yet, and the
        // frame would be missing whatever was still in the queue.
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

/// Composite a decoded picture onto `dc` — here because a source `HBITMAP` is a GDI object.
/// `pixels` is `src_w * src_h * 4` bytes of **premultiplied** BGRA, top-down — WIC's
/// `GUID_WICPixelFormat32bppPBGRA`, which is why `image.rs` asks for it. `AlphaBlend` resamples
/// when the sizes differ, so no resampler lives here.
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

/// Put an opaque picture onto a **printer's** `dc`, scaled into `dest` (left, top, width, height
/// in device pixels) — a print job's page. `StretchDIBits`, since a printer driver need not
/// support `AlphaBlend`, and paper has no alpha to blend. `pixels` is BGRA, top-down. No GDI
/// object is created.
pub fn print_image(dc: HDC, dest: (i32, i32, i32, i32), size: (u32, u32), pixels: &[u8]) -> bool {
    use windows::Win32::Graphics::Gdi::{SRCCOPY, StretchDIBits};
    let (Ok(w), Ok(h)) = (i32::try_from(size.0), i32::try_from(size.1)) else {
        return false;
    };
    if w <= 0 || h <= 0 || pixels.len() < (size.0 as usize) * (size.1 as usize) * 4 {
        return false;
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).expect("forty"),
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: `pixels` holds the whole image and `info` describes it, both for the call.
    unsafe {
        StretchDIBits(
            dc,
            dest.0,
            dest.1,
            dest.2,
            dest.3,
            0,
            0,
            w,
            h,
            Some(pixels.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        ) != 0
    }
}
