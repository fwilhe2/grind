// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Device-independent bitmaps, the masks an icon is drawn with, and clips over a bitmap.

use super::{Bytes, Rgb};

/// The part of a bitmap a record copies: `x`, `y`, `width` and `height` in its pixels, from
/// its top-left corner when `from_top`, else — as `StretchDIBits` counts in a bottom-up DIB —
/// from its bottom-left.
#[derive(Clone, Copy, Debug)]
pub(super) struct Source {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) from_top: bool,
}

/// A bitmap read out of a DIB: its pixels, or the JPEG or PNG it carries whole.
pub(super) enum Dib {
    Pixels(tiny_skia::Pixmap),
    Encoded(&'static str, Vec<u8>),
}

impl Dib {
    /// Only the part `source` copies, when it is within the bitmap and less than all of it.
    pub(super) fn cut(self, source: Option<Source>) -> Self {
        let (Dib::Pixels(pixels), Some(source)) = (&self, source) else {
            return self;
        };
        let (width, height) = (pixels.width() as i32, pixels.height() as i32);
        let y = if source.from_top {
            source.y
        } else {
            height - source.y - source.height
        };
        let whole = source.x == 0 && y == 0 && source.width == width && source.height == height;
        let inside = source.x >= 0
            && y >= 0
            && source.width > 0
            && source.height > 0
            && source.x + source.width <= width
            && y + source.height <= height;
        if whole || !inside {
            return self;
        }
        tiny_skia::IntRect::from_xywh(source.x, y, source.width as u32, source.height as u32)
            .and_then(|rect| pixels.clone_rect(rect))
            .map_or(self, Dib::Pixels)
    }

    /// As a picture's MIME type and bytes.
    pub(super) fn encoded(self) -> Option<(&'static str, Vec<u8>)> {
        match self {
            Dib::Pixels(pixels) => Some(("image/png", pixels.encode_png().ok()?)),
            Dib::Encoded(mime, data) => Some((mime, data)),
        }
    }
}

/// `pixels` made transparent where `mask`, stretched over it, is light.
pub(super) fn masked(pixels: &mut tiny_skia::Pixmap, mask: &tiny_skia::Pixmap) {
    let (width, height) = (pixels.width(), pixels.height());
    let (mw, mh) = (mask.width(), mask.height());
    let source = mask.data();
    for (index, pixel) in pixels
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .enumerate()
    {
        let (x, y) = (index as u32 % width, index as u32 / width);
        let at = ((y * mh / height) * mw + x * mw / width) as usize * 4;
        let light = source
            .get(at..at + 3)
            .is_some_and(|rgb| rgb.iter().map(|c| u32::from(*c)).sum::<u32>() > 3 * 127);
        if light {
            *pixel = [0, 0, 0, 0];
        }
    }
}

/// A PNG drawn into `rect` made transparent outside every path of `chain` (path data in the
/// picture's units), or `None` when it does not decode.
pub(super) fn clipped(png: &[u8], rect: [f64; 4], chain: &[(&str, bool)]) -> Option<Vec<u8>> {
    let mut pixels = tiny_skia::Pixmap::decode_png(png).ok()?;
    let (width, height) = (pixels.width(), pixels.height());
    let to_pixels = tiny_skia::Transform::from_translate(-rect[0] as f32, -rect[1] as f32)
        .post_scale(
            width as f32 / rect[2] as f32,
            height as f32 / rect[3] as f32,
        );
    for (d, evenodd) in chain {
        let path = path_of(d)?;
        let mut mask = tiny_skia::Mask::new(width, height)?;
        let rule = if *evenodd {
            tiny_skia::FillRule::EvenOdd
        } else {
            tiny_skia::FillRule::Winding
        };
        // Cut-outs side by side would each leave a seam where they meet if smoothed.
        mask.fill_path(&path, rule, !*evenodd, to_pixels);
        pixels.apply_mask(&mask);
    }
    pixels.encode_png().ok()
}

/// Path data as this module writes it — `M`, `L`, `C` and `Z` with plain numbers — as a path.
pub(super) fn path_of(d: &str) -> Option<tiny_skia::Path> {
    let mut builder = tiny_skia::PathBuilder::new();
    let mut numbers: Vec<f32> = Vec::new();
    let mut command = ' ';
    let flush = |command: char, numbers: &mut Vec<f32>, builder: &mut tiny_skia::PathBuilder| {
        match (command, numbers.as_slice()) {
            ('M', [x, y]) => builder.move_to(*x, *y),
            ('L', [x, y]) => builder.line_to(*x, *y),
            ('C', [a, b, c, d, e, f]) => builder.cubic_to(*a, *b, *c, *d, *e, *f),
            _ => {}
        }
        numbers.clear();
    };
    for token in d.split_whitespace() {
        let mut rest = token;
        while let Some(first) = rest.chars().next()
            && first.is_ascii_alphabetic()
        {
            flush(command, &mut numbers, &mut builder);
            command = first;
            if first == 'Z' {
                builder.close();
            }
            rest = &rest[1..];
        }
        if let Ok(value) = rest.parse::<f32>() {
            numbers.push(value);
            let wanted = if command == 'C' { 6 } else { 2 };
            if numbers.len() == wanted {
                flush(command, &mut numbers, &mut builder);
            }
        }
    }
    builder.finish()
}

/// A device-independent bitmap — a `BITMAPINFOHEADER` or `BITMAPCOREHEADER`, its colour table
/// and its rows, the bits after the table unless given apart — or the JPEG or PNG it carries
/// whole. `None` for compression this does not read (RLE).
pub(super) fn dib(info: &[u8], bits: Option<&[u8]>) -> Option<Dib> {
    let b = Bytes(info);
    let size = b.u32(0)? as usize;
    let (width, height, depth, compression, colors, table) = if size == 12 {
        (
            i32::from(b.u16(4)?),
            i32::from(b.u16(6)?),
            b.u16(10)?,
            0,
            0,
            3,
        )
    } else {
        (
            b.i32(4)?,
            b.i32(8)?,
            b.u16(14)?,
            b.u32(16)?,
            b.u32(32)? as usize,
            4,
        )
    };
    let colors = match (colors, depth) {
        (0, 1 | 4 | 8) => 1 << depth,
        (n, _) => n.min(256),
    };
    let masks = if compression == 3 && size == 40 {
        12
    } else {
        0
    };
    let table_at = size;
    let bits_at = table_at + masks + colors * table;
    let bits = match bits {
        Some(bits) => bits,
        None => info.get(bits_at..)?,
    };
    match compression {
        4 => return Some(Dib::Encoded("image/jpeg", bits.to_vec())),
        5 => return Some(Dib::Encoded("image/png", bits.to_vec())),
        0 | 3 => {}
        _ => return None,
    }
    let (w, h) = (width.unsigned_abs(), height.unsigned_abs());
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 64 << 20 {
        return None;
    }
    let palette: Vec<Rgb> = (0..colors)
        .map(|i| {
            let at = table_at + masks + i * table;
            Rgb(
                *info.get(at + 2).unwrap_or(&0),
                *info.get(at + 1).unwrap_or(&0),
                *info.get(at).unwrap_or(&0),
            )
        })
        .collect();
    let mask = |i: usize| b.u32(size + i * 4);
    let fields = if compression == 3 {
        match (mask(0), mask(1), mask(2)) {
            (Some(r), Some(g), Some(b)) => Some([r, g, b]),
            _ => Some(if depth == 16 {
                [0x7C00, 0x03E0, 0x001F]
            } else {
                [0xFF_0000, 0xFF00, 0xFF]
            }),
        }
    } else if depth == 16 {
        Some([0x7C00, 0x03E0, 0x001F])
    } else {
        None
    };
    let stride = (w as usize * usize::from(depth)).div_ceil(32) * 4;
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    let out = pixmap.data_mut();
    let channel = |value: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        (((value & mask) >> shift) * 255 / max.max(1)) as u8
    };
    for row in 0..h as usize {
        // Bottom-up unless the height is negative.
        let source = if height > 0 {
            h as usize - 1 - row
        } else {
            row
        };
        let line = bits.get(source * stride..source * stride + stride)?;
        for x in 0..w as usize {
            let rgb = match depth {
                1 | 4 | 8 => {
                    let bit = x * usize::from(depth);
                    let byte = line[bit / 8];
                    let index =
                        (byte >> (8 - usize::from(depth) - bit % 8)) & ((1u16 << depth) - 1) as u8;
                    *palette.get(usize::from(index)).unwrap_or(&Rgb::BLACK)
                }
                16 => {
                    let value = u32::from(u16::from_le_bytes([line[x * 2], line[x * 2 + 1]]));
                    let [r, g, bl] = fields.unwrap_or_default();
                    Rgb(channel(value, r), channel(value, g), channel(value, bl))
                }
                24 => Rgb(line[x * 3 + 2], line[x * 3 + 1], line[x * 3]),
                32 => {
                    let value = u32::from_le_bytes(line[x * 4..x * 4 + 4].try_into().ok()?);
                    match fields {
                        Some([r, g, bl]) => {
                            Rgb(channel(value, r), channel(value, g), channel(value, bl))
                        }
                        None => Rgb(line[x * 4 + 2], line[x * 4 + 1], line[x * 4]),
                    }
                }
                _ => return None,
            };
            let at = (row * w as usize + x) * 4;
            out[at..at + 4].copy_from_slice(&[rgb.0, rgb.1, rgb.2, 255]);
        }
    }
    Some(Dib::Pixels(pixmap))
}
