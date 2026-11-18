// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Windows metafiles: an optional placeable header, then records of a size in 16-bit words.

use super::*;

/// A Windows metafile, with or without the placeable header that gives its bounds: records
/// of a size in 16-bit words, a function and parameters, many of them in reverse order.
pub(super) fn wmf(data: &[u8], fonts: &Fonts) -> Option<Metafile> {
    let b = Bytes(data);
    let placeable = b.u32(0)? == 0x9AC6_CDD7;
    let bounds = if placeable {
        Some((
            f64::from(b.i16(6)?),
            f64::from(b.i16(8)?),
            f64::from(b.i16(10)?),
            f64::from(b.i16(12)?),
        ))
    } else {
        None
    };
    let start = if placeable { 22 } else { 0 };
    let header = Bytes(data.get(start..)?);
    if !matches!(header.u16(0)?, 1 | 2) || header.u16(2)? != 9 {
        return None;
    }
    let mut canvas = Canvas::new(fonts);
    let (width, height) = match bounds {
        Some((left, top, right, bottom)) => {
            canvas.state.map.window = ((left, top), (right - left, bottom - top));
            ((right - left).abs(), (bottom - top).abs())
        }
        None => (0.0, 0.0),
    };
    canvas.state.map.wmf = Some((width, height));
    // The window as the records leave it, for a metafile with no bounds of its own.
    let mut at = start + usize::from(header.u16(2)?) * 2;
    let mut count = 0usize;
    let mut window_ext = None;
    while at + 6 <= data.len() && count < 1 << 20 {
        count += 1;
        let size = b.u32(at)? as usize * 2;
        let function = b.u16(at + 4)?;
        if size < 6 || at + size > data.len() || function == 0 {
            break;
        }
        let record = Bytes(&data[at + 6..at + size]);
        if function == 0x020C {
            window_ext = Some((f64::from(record.i16(2)?), f64::from(record.i16(0)?)));
        }
        if bounds.is_none()
            && let Some((x, y)) = window_ext
        {
            canvas.state.map.wmf = Some((x.abs(), y.abs()));
        }
        wmf_record(&mut canvas, function, &record);
        at += size;
    }
    let (width, height) = canvas.state.map.wmf.unwrap_or((width, height));
    canvas.finish(width, height)
}

fn wmf_record(canvas: &mut Canvas, function: u16, r: &Bytes<'_>) -> Option<()> {
    let w = |i: usize| r.i16(i * 2).map(f64::from);
    let points = |count: usize, at: usize| -> Option<Vec<(f64, f64)>> {
        (0..count.min(1 << 16))
            .map(|i| Some((w(at + i * 2)?, w(at + i * 2 + 1)?)))
            .collect()
    };
    let state = &mut canvas.state;
    match function {
        0x001E => canvas.save(),
        0x0127 => canvas.restore(i32::from(r.i16(0)?)),
        0x0102 => state.opaque = r.u16(0)? == 2,
        0x0106 => state.evenodd = r.u16(0)? != 2,
        0x012E => state.align = u32::from(r.u16(0)?),
        0x0209 => state.text = r.color(0)?,
        0x0201 => state.background = r.color(0)?,
        0x020B => state.map.window.0 = (w(1)?, w(0)?),
        0x020C => state.map.window.1 = (w(1)?, w(0)?),
        0x0214 => canvas.move_to((w(1)?, w(0)?)),
        0x0213 => canvas.line_to((w(1)?, w(0)?)),
        0x012D => {
            let index = usize::from(r.u16(0)?);
            if let Some(Some(object)) = canvas.objects.get(index).cloned() {
                canvas.select(&object);
            }
        }
        0x01F0 => {
            if let Some(slot) = canvas.objects.get_mut(usize::from(r.u16(0)?)) {
                *slot = None;
            }
        }
        0x02FA => {
            let pen = pen(u32::from(r.u16(0)?), w(1)?, r.color(6)?);
            wmf_create(&mut canvas.objects, Object::Pen(pen));
        }
        0x02FC => {
            let brush = brush(u32::from(r.u16(0)?), r.color(2)?);
            wmf_create(&mut canvas.objects, Object::Brush(brush));
        }
        0x02FB => {
            let face: String =
                r.0.get(18..)
                    .unwrap_or_default()
                    .iter()
                    .take_while(|b| **b != 0)
                    .map(|b| char::from(*b))
                    .collect();
            let font = Font {
                height: w(0)?,
                escapement: w(2)?,
                weight: i32::from(r.i16(8)?),
                italic: r.0.get(10).is_some_and(|b| *b != 0),
                underline: r.0.get(11).is_some_and(|b| *b != 0),
                family: *r.0.get(17).unwrap_or(&0),
                face,
            };
            wmf_create(&mut canvas.objects, Object::Font(font));
        }
        // CreatePalette, CreatePatternBrush, CreateRegion, DIBCreatePatternBrush: a slot each.
        0x00F7 | 0x01F9 | 0x06FF | 0x0142 => wmf_create(&mut canvas.objects, Object::Other),
        0x0324 => {
            let count = usize::from(r.u16(0)?);
            canvas.polygon(&points(count, 1)?);
        }
        0x0325 => {
            let count = usize::from(r.u16(0)?);
            canvas.polyline(&points(count, 1)?);
        }
        0x0538 => {
            let polys = usize::from(r.u16(0)?);
            let mut at = 1 + polys;
            let mut d = String::new();
            for i in 0..polys {
                let count = usize::from(r.u16((1 + i) * 2)?);
                d.push_str(&canvas.data(&points(count, at)?, true));
                at += count * 2;
            }
            canvas.shape(&d, true);
        }
        0x041B | 0x061C => canvas.rectangle((w(3)?, w(2)?), (w(1)?, w(0)?)),
        0x0416 => canvas.clip_rect((w(3)?, w(2)?), (w(1)?, w(0)?)),
        0x0415 => canvas.exclude((w(3)?, w(2)?), (w(1)?, w(0)?)),
        // A region this does not keep: the clip it selects is taken as none.
        0x012C => canvas.state.clip = None,
        0x0418 => canvas.ellipse((w(3)?, w(2)?), (w(1)?, w(0)?)),
        0x0521 => {
            let count = usize::from(r.u16(0)?);
            let text = latin1(r.0.get(2..2 + count)?);
            let after = 1 + count.div_ceil(2);
            canvas.text((w(after + 1)?, w(after)?), &text, None);
        }
        0x0A32 => {
            let at = (w(1)?, w(0)?);
            let count = usize::from(r.u16(4)?);
            let options = r.u16(6)?;
            let mut offset = 8;
            if options & 0x6 != 0 {
                if options & 0x2 != 0 {
                    let rect = |i: usize| r.i16(offset + i * 2).map(f64::from);
                    let (left, top, right, bottom) = (rect(0)?, rect(1)?, rect(2)?, rect(3)?);
                    let background = canvas.state.background;
                    canvas.block((left, top), (right, bottom), background);
                }
                offset += 8;
            }
            let text = latin1(r.0.get(offset..offset + count)?);
            let dx_at = offset + count + count % 2;
            let dx: Option<Vec<f64>> = (0..count)
                .map(|i| r.i16(dx_at + i * 2).map(f64::from))
                .collect();
            canvas.text(at, &text, dx.as_deref());
        }
        0x061D => {
            let rop = r.u32(0)?;
            let (height, width, y, x) = (w(2)?, w(3)?, w(4)?, w(5)?);
            canvas.pattern(rop, [(x, y), (x + width, y + height)]);
        }
        0x0B41 | 0x0F43 | 0x0940 => wmf_blt(canvas, function, r)?,
        _ => {}
    }
    Some(())
}

/// `META_DIBSTRETCHBLT`, `META_STRETCHDIB` and `META_DIBBITBLT`: a bitmap copied into a
/// rectangle, or with no bitmap a fill by raster operation.
fn wmf_blt(canvas: &mut Canvas, function: u16, r: &Bytes<'_>) -> Option<()> {
    let w = |i: usize| r.i16(i * 2).map(f64::from);
    let rop = r.u32(0)?;
    let i = |at: usize| r.i16(at * 2).map(i32::from);
    // Words after the raster operation: the destination's corner and size, the source's, and
    // where the DIB starts, by function. `META_STRETCHDIB` counts its source as `StretchDIBits`
    // does; the others copy from a device context, from its top.
    let (dest, size, source, dib) = match function {
        0x0F43 => (
            (w(10)?, w(9)?),
            (w(8)?, w(7)?),
            (i(6)?, i(5)?, i(4)?, i(3)?, false),
            22,
        ),
        0x0B41 => (
            (w(9)?, w(8)?),
            (w(7)?, w(6)?),
            (i(5)?, i(4)?, i(3)?, i(2)?, true),
            20,
        ),
        _ => (
            (w(7)?, w(6)?),
            (w(5)?, w(4)?),
            (i(3)?, i(2)?, i(5)?, i(4)?, true),
            16,
        ),
    };
    let (x, y, width, height, from_top) = source;
    let source = Source {
        x,
        y,
        width,
        height,
        from_top,
    };
    let corners = [dest, (dest.0 + size.0, dest.1 + size.1)];
    match r.0.get(dib..) {
        Some(info) if info.len() > 12 => canvas.blit((rop, Some(source)), info, None, corners),
        _ => canvas.pattern(rop, corners),
    }
    Some(())
}

/// A WMF object takes the lowest free slot.
fn wmf_create(objects: &mut Vec<Option<Object>>, object: Object) {
    match objects.iter().position(Option::is_none) {
        Some(at) => objects[at] = Some(object),
        None if objects.len() < 1 << 16 => objects.push(Some(object)),
        None => {}
    }
}
