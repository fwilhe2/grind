// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Decoding an inserted picture's bytes into pixels — WIC, and the one namespace here that is
//! not GDI or a dialog.
//!
//! `ui_text_gtk`'s `texture_of` reaches gdk-pixbuf's loaders through `gdk::Texture::from_bytes`;
//! this is the same question asked of the platform this shell is on. **Decodes on every call**
//! rather than caching — `ui_text_gtk`'s own `ponytail` on `texture_of` names the same trade,
//! and the trigger for a cache is the same one: a document with a large picture whose repaint
//! cost becomes visible, not before.

use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapFrameDecode,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IStream};
use windows::Win32::UI::Shell::SHCreateMemStream;

use crate::gdi;

/// A decoded picture: premultiplied BGRA, top-down, four bytes a pixel and no padding — exactly
/// what [`gdi::blit_image`] wants and what `AlphaBlend`'s `BLENDFUNCTION` requires of its source.
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Wrap `data` in the `IStream` WIC's decoder wants — `SHCreateMemStream` copies the bytes into
/// its own buffer, so this owns nothing of the caller's past this call.
fn stream(data: &[u8]) -> Option<IStream> {
    // SAFETY: `data` is a valid slice for the length of this call, which is all
    // `SHCreateMemStream` needs — it copies the bytes rather than borrowing them.
    unsafe { SHCreateMemStream(Some(data)) }
}

/// The one frame this build reads — index 0, which is every still image and the first frame of
/// an animated one. WIC has no opinion about a corrupt file or a format nobody's decoder
/// understands beyond `Err`, which becomes `None` here the same way a bad element in the XML
/// becomes `Ignore` rather than a failure (R5's tolerance, over a picture instead of markup).
fn frame(factory: &IWICImagingFactory, data: &[u8]) -> Option<IWICBitmapFrameDecode> {
    let stream = stream(data)?;
    // SAFETY: `factory` and `stream` are live COM objects; the decoder and frame are owned
    // results dropped normally at scope end.
    unsafe {
        let decoder = factory
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .ok()?;
        decoder.GetFrame(0).ok()
    }
}

/// The one COM object this module needs, created fresh per call — this shell's `dialog.rs`
/// creates its own `IFileOpenDialog` the same way rather than keeping one alive, and a picture
/// is inserted or drawn far less often than a keystroke.
fn factory() -> Option<IWICImagingFactory> {
    // SAFETY: COM is initialised for as long as the window is (`win.rs`'s `_com` guard), which
    // covers every caller of this module.
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok() }
}

/// A picture's natural size in pixels, without paying for the pixels themselves — what
/// `text::geom::flow_of`'s picture hook needs to measure a block, and nothing more.
pub fn size(data: &[u8]) -> Option<(u32, u32)> {
    let factory = factory()?;
    let frame = frame(&factory, data)?;
    let (mut w, mut h) = (0u32, 0u32);
    // SAFETY: `frame` is live; `w`/`h` are valid out-parameters for the length of the call.
    unsafe { frame.GetSize(&mut w, &mut h).ok()? };
    (w > 0 && h > 0).then_some((w, h))
}

/// Decode `data` to premultiplied BGRA — what actually drawing the picture needs.
///
/// The format converter is WIC's own answer to "whatever this file's native pixel format is,
/// give me the one GDI's `AlphaBlend` wants" — a paletted GIF, a CMYK JPEG and an indexed PNG
/// all come out the same shape, so nothing above this reads a format tag.
pub fn decode(data: &[u8]) -> Option<Decoded> {
    let factory = factory()?;
    let frame = frame(&factory, data)?;
    // SAFETY: `factory` and `frame` are live; the converter is an owned result.
    let converter = unsafe { factory.CreateFormatConverter().ok()? };
    // SAFETY: every argument outlives the call, which is synchronous.
    unsafe {
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .ok()?;
    }
    let (mut w, mut h) = (0u32, 0u32);
    // SAFETY: `converter` is live; `w`/`h` are valid out-parameters.
    unsafe { converter.GetSize(&mut w, &mut h).ok()? };
    if w == 0 || h == 0 {
        return None;
    }
    let stride = w.checked_mul(4)?;
    let mut pixels = vec![0u8; (stride as usize).checked_mul(h as usize)?];
    // SAFETY: `pixels` is exactly `stride * h` bytes, which is what `CopyPixels` is told and
    // what a full-frame, no-clip-rectangle copy writes.
    unsafe {
        converter
            .CopyPixels(std::ptr::null(), stride, &mut pixels)
            .ok()?
    };
    Some(Decoded {
        width: w,
        height: h,
        pixels,
    })
}

/// The MIME type to store on the frame, guessed from the file's own extension — this shell has
/// no GIO `content_type_guess` to ask the bytes themselves, so the extension is the whole
/// answer, same list `dialog::open_image_path`'s filter offers.
pub fn mime_of(path: &std::path::Path) -> String {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("tif" | "tiff") => "image/tiff",
        _ => "image/png",
    }
    .to_owned()
}

/// Draw a decoded picture into `dest` of `dc`, scaled by `AlphaBlend` rather than by this
/// module — the source and destination rectangles may differ in size, and letting GDI do the
/// resampling is one function call instead of a resampler this crate would then own.
pub fn draw(dc: HDC, dest: crate::sheet::geom::Rect, image: &Decoded) -> bool {
    gdi::blit_image(dc, dest, (image.width, image.height), &image.pixels)
}
