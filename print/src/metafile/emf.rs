// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Enhanced metafiles: `EMR_HEADER`, then records of a type and a size in bytes.

use super::*;

/// An enhanced metafile: `EMR_HEADER` then records, each a type and a size in bytes.
pub(super) fn emf(data: &[u8], fonts: &Fonts) -> Option<Metafile> {
    let b = Bytes(data);
    let frame = (b.i32(24)?, b.i32(28)?, b.i32(32)?, b.i32(36)?);
    let device = (f64::from(b.i32(72)?), f64::from(b.i32(76)?));
    let millimetres = (f64::from(b.i32(80)?), f64::from(b.i32(84)?));
    let per_mm = (
        device.0 / nonzero(millimetres.0),
        device.1 / nonzero(millimetres.1),
    );
    // The frame, in hundredths of a millimetre, is the picture; in device units here.
    let left = f64::from(frame.0) / 100.0 * per_mm.0;
    let top = f64::from(frame.1) / 100.0 * per_mm.1;
    let width = f64::from(frame.2 - frame.0) / 100.0 * per_mm.0;
    let height = f64::from(frame.3 - frame.1) / 100.0 * per_mm.1;
    let mut canvas = Canvas::new(fonts);
    canvas.state.map.per_mm = per_mm;
    canvas.state.map.origin = (left, top);
    let mut at = 0usize;
    let mut count = 0usize;
    while at + 8 <= data.len() && count < 1 << 20 {
        count += 1;
        let kind = b.u32(at)?;
        let size = b.u32(at + 4)? as usize;
        if size < 8 || at + size > data.len() {
            break;
        }
        let record = Bytes(&data[at..at + size]);
        if kind == 14 {
            break;
        }
        emf_record(&mut canvas, kind, &record);
        at += size;
    }
    canvas.finish(width, height)
}

fn emf_record(canvas: &mut Canvas, kind: u32, r: &Bytes<'_>) -> Option<()> {
    let point32 = |at: usize| Some((f64::from(r.i32(at)?), f64::from(r.i32(at + 4)?)));
    let point16 = |at: usize| Some((f64::from(r.i16(at)?), f64::from(r.i16(at + 2)?)));
    let points16 = |count: usize, at: usize| -> Option<Vec<(f64, f64)>> {
        (0..count.min(1 << 20))
            .map(|i| point16(at + i * 4))
            .collect()
    };
    let points32 = |count: usize, at: usize| -> Option<Vec<(f64, f64)>> {
        (0..count.min(1 << 20))
            .map(|i| point32(at + i * 8))
            .collect()
    };
    let state = &mut canvas.state;
    match kind {
        // EMR_POLYBEZIER, EMR_POLYGON, EMR_POLYLINE, EMR_POLYBEZIERTO, EMR_POLYLINETO.
        2..=6 => {
            let points = points32(r.u32(24)? as usize, 28)?;
            emf_poly(canvas, kind, &points);
        }
        // EMR_POLYPOLYLINE, EMR_POLYPOLYGON.
        7 | 8 => emf_polypoly(canvas, kind == 8, r, false)?,
        9 => state.map.window.1 = point32(8)?,
        10 => state.map.window.0 = point32(8)?,
        11 => state.map.viewport.1 = point32(8)?,
        12 => state.map.viewport.0 = point32(8)?,
        17 => state.map.mode = r.u32(8)?,
        18 => state.opaque = r.u32(8)? == 2,
        19 => state.evenodd = r.u32(8)? != 2,
        22 => state.align = r.u32(8)?,
        24 => state.text = r.color(8)?,
        25 => state.background = r.color(8)?,
        27 => canvas.move_to(point32(8)?),
        33 => canvas.save(),
        34 => canvas.restore(r.i32(8)?),
        35 => {
            let m = |i: usize| r.f32(8 + i * 4).map(f64::from);
            state.map.world = [m(0)?, m(1)?, m(2)?, m(3)?, m(4)?, m(5)?];
        }
        36 => {
            let m = |i: usize| r.f32(8 + i * 4).map(f64::from);
            let given = [m(0)?, m(1)?, m(2)?, m(3)?, m(4)?, m(5)?];
            state.map.world = match r.u32(32)? {
                1 => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                2 => compose(given, state.map.world),
                3 => compose(state.map.world, given),
                4 => given,
                _ => state.map.world,
            };
        }
        37 => {
            let index = r.u32(8)?;
            if index & 0x8000_0000 != 0 {
                canvas.stock(index & 0x7FFF_FFFF);
            } else if let Some(Some(object)) = canvas.objects.get(index as usize).cloned() {
                canvas.select(&object);
            }
        }
        38 => {
            let pen = pen(r.u32(12)?, f64::from(r.i32(16)?), r.color(24)?);
            put(&mut canvas.objects, r.u32(8)?, Object::Pen(pen));
        }
        39 => {
            let brush = brush(r.u32(12)?, r.color(16)?);
            put(&mut canvas.objects, r.u32(8)?, Object::Brush(brush));
        }
        40 => {
            if let Some(slot) = canvas.objects.get_mut(r.u32(8)? as usize) {
                *slot = None;
            }
        }
        29 => canvas.exclude(point32(8)?, point32(16)?),
        30 => canvas.clip_rect(point32(8)?, point32(16)?),
        42 => canvas.ellipse(point32(8)?, point32(16)?),
        43 | 44 => canvas.rectangle(point32(8)?, point32(16)?),
        54 => canvas.line_to(point32(8)?),
        59 => canvas.path = Some(String::new()),
        61 => {
            if let Some(path) = &mut canvas.path {
                path.push('Z');
            }
        }
        62 => canvas.draw_path(true, false),
        63 => canvas.draw_path(true, true),
        64 => canvas.draw_path(false, true),
        67 => {
            if let Some(d) = canvas.path.take() {
                // RGN_COPY (5) replaces the clip; the others are taken as its intersection.
                let within = r.u32(8)? != 5;
                canvas.clip(d, within);
            }
        }
        68 => canvas.path = None,
        75 => emf_clip_region(canvas, r)?,
        76 | 77 => emf_blt(canvas, kind, r)?,
        81 => {
            let dest = (f64::from(r.i32(24)?), f64::from(r.i32(28)?));
            let size = (f64::from(r.i32(72)?), f64::from(r.i32(76)?));
            let source = Source {
                x: r.i32(32)?,
                y: r.i32(36)?,
                width: r.i32(40)?,
                height: r.i32(44)?,
                from_top: false,
            };
            let (off_bmi, cb_bmi) = (r.u32(48)? as usize, r.u32(52)? as usize);
            let (off_bits, cb_bits) = (r.u32(56)? as usize, r.u32(60)? as usize);
            let rop = r.u32(68)?;
            let info = r.0.get(off_bmi..off_bmi + cb_bmi)?;
            let bits = r.0.get(off_bits..off_bits + cb_bits)?;
            canvas.blit(
                (rop, Some(source)),
                info,
                Some(bits),
                [dest, (dest.0 + size.0, dest.1 + size.1)],
            );
        }
        82 => {
            let font = Font {
                height: f64::from(r.i32(12)?),
                escapement: f64::from(r.i32(20)?),
                weight: r.i32(28)?,
                italic: r.0.get(32).is_some_and(|b| *b != 0),
                underline: r.0.get(33).is_some_and(|b| *b != 0),
                family: *r.0.get(39)?,
                face: utf16(r.0.get(40..104)?),
            };
            put(&mut canvas.objects, r.u32(8)?, Object::Font(font));
        }
        83 => emf_text(canvas, r, false)?,
        84 => emf_text(canvas, r, true)?,
        // EMR_POLYBEZIER16 … EMR_POLYLINETO16.
        85..=89 => {
            let points = points16(r.u32(24)? as usize, 28)?;
            emf_poly(canvas, kind - 83, &points);
        }
        90 | 91 => emf_polypoly(canvas, kind == 91, r, true)?,
        95 => {
            // LOGPENEX: a style, a width and a brush's style and colour.
            let style = r.u32(28)?;
            let width = f64::from(r.u32(32)?);
            let pen = if r.u32(36)? == 1 {
                Pen::None
            } else {
                pen(style & 0x0F, width, r.color(40)?)
            };
            put(&mut canvas.objects, r.u32(8)?, Object::Pen(pen));
        }
        114 => {
            let dest = (f64::from(r.i32(24)?), f64::from(r.i32(28)?));
            let size = (f64::from(r.i32(32)?), f64::from(r.i32(36)?));
            let (off_bmi, cb_bmi) = (r.u32(84)? as usize, r.u32(88)? as usize);
            let (off_bits, cb_bits) = (r.u32(92)? as usize, r.u32(96)? as usize);
            let info = r.0.get(off_bmi..off_bmi + cb_bmi)?;
            let bits = r.0.get(off_bits..off_bits + cb_bits)?;
            let source = Source {
                x: r.i32(44)?,
                y: r.i32(48)?,
                width: r.i32(100)?,
                height: r.i32(104)?,
                from_top: true,
            };
            canvas.blit(
                (0x00CC_0020, Some(source)),
                info,
                Some(bits),
                [dest, (dest.0 + size.0, dest.1 + size.1)],
            );
        }
        _ => {}
    }
    Some(())
}

/// The record types 2–6 as one: 2 Béziers, 3 a polygon, 4 a polyline, 5 Béziers and 6 lines
/// from the current position.
fn emf_poly(canvas: &mut Canvas, kind: u32, points: &[(f64, f64)]) {
    match kind {
        2 => canvas.beziers(points, false),
        3 => canvas.polygon(points),
        4 => canvas.polyline(points),
        5 => canvas.beziers(points, true),
        6 => {
            for &point in points {
                canvas.line_to(point);
            }
        }
        _ => {}
    }
}

fn emf_polypoly(canvas: &mut Canvas, closed: bool, r: &Bytes<'_>, short: bool) -> Option<()> {
    let polys = r.u32(24)? as usize;
    let mut at = 32 + polys.min(1 << 16) * 4;
    let mut d = String::new();
    for i in 0..polys.min(1 << 16) {
        let count = r.u32(32 + i * 4)? as usize;
        let points: Vec<(f64, f64)> = (0..count.min(1 << 20))
            .map(|j| {
                if short {
                    Some((
                        f64::from(r.i16(at + j * 4)?),
                        f64::from(r.i16(at + j * 4 + 2)?),
                    ))
                } else {
                    Some((
                        f64::from(r.i32(at + j * 8)?),
                        f64::from(r.i32(at + j * 8 + 4)?),
                    ))
                }
            })
            .collect::<Option<_>>()?;
        at += count * if short { 4 } else { 8 };
        d.push_str(&canvas.data(&points, closed));
    }
    canvas.shape(&d, closed);
    Some(())
}

fn emf_blt(canvas: &mut Canvas, kind: u32, r: &Bytes<'_>) -> Option<()> {
    let dest = (f64::from(r.i32(24)?), f64::from(r.i32(28)?));
    let size = (f64::from(r.i32(32)?), f64::from(r.i32(36)?));
    let corners = [dest, (dest.0 + size.0, dest.1 + size.1)];
    let rop = r.u32(40)?;
    let (off_bmi, cb_bmi) = (r.u32(84)? as usize, r.u32(88)? as usize);
    let (off_bits, cb_bits) = (r.u32(92)? as usize, r.u32(96)? as usize);
    if cb_bmi == 0 {
        canvas.pattern(rop, corners);
        return Some(());
    }
    // BitBlt copies as much as it draws; StretchBlt says how much.
    let (width, height) = if kind == 77 {
        (r.i32(100)?, r.i32(104)?)
    } else {
        (r.i32(32)?, r.i32(36)?)
    };
    let source = Source {
        x: r.i32(44)?,
        y: r.i32(48)?,
        width,
        height,
        from_top: true,
    };
    let info = r.0.get(off_bmi..off_bmi + cb_bmi)?;
    let bits = r.0.get(off_bits..off_bits + cb_bits)?;
    canvas.blit((rop, Some(source)), info, Some(bits), corners);
    Some(())
}

/// `EMR_EXTTEXTOUTW` and `EMR_EXTTEXTOUTA`: a reference point, a UTF-16 or 8-bit string and
/// its advances.
fn emf_text(canvas: &mut Canvas, r: &Bytes<'_>, wide: bool) -> Option<()> {
    let at = (f64::from(r.i32(36)?), f64::from(r.i32(40)?));
    let count = r.u32(44)? as usize;
    let off_string = r.u32(48)? as usize;
    let options = r.u32(52)?;
    // ETO_GLYPH_INDEX: glyph numbers in a font this does not have, not characters.
    if options & 0x10 != 0 || count > 1 << 16 {
        return Some(());
    }
    let text = if wide {
        utf16(r.0.get(off_string..off_string + count * 2)?)
    } else {
        latin1(r.0.get(off_string..off_string + count)?)
    };
    let off_dx = r.u32(72)? as usize;
    let pairs = options & 0x2000 != 0;
    let dx: Option<Vec<f64>> = (off_dx > 0)
        .then(|| {
            (0..count)
                .map(|i| r.i32(off_dx + i * if pairs { 8 } else { 4 }).map(f64::from))
                .collect()
        })
        .flatten();
    if options & 0x2 != 0 {
        let rect = (
            (f64::from(r.i32(56)?), f64::from(r.i32(60)?)),
            (f64::from(r.i32(64)?), f64::from(r.i32(68)?)),
        );
        if rect.0 != rect.1 {
            let background = canvas.state.background;
            canvas.block(rect.0, rect.1, background);
        }
    }
    canvas.text(at, &text, dx.as_deref());
    Some(())
}

/// `a` after `b`, as `XFORM`s compose.
fn compose(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn put(objects: &mut Vec<Option<Object>>, index: u32, object: Object) {
    let index = index as usize;
    if index > 1 << 16 {
        return;
    }
    if objects.len() <= index {
        objects.resize(index + 1, None);
    }
    objects[index] = Some(object);
}

/// `EMR_EXTSELECTCLIPRGN`: a region's rectangles in device units, or none — with `RGN_COPY`,
/// no clip at all.
fn emf_clip_region(canvas: &mut Canvas, r: &Bytes<'_>) -> Option<()> {
    let size = r.u32(8)? as usize;
    let mode = r.u32(12)?;
    if size < 32 {
        if mode == 5 {
            canvas.state.clip = None;
        }
        return Some(());
    }
    let count = r.u32(16 + 8)? as usize;
    let origin = canvas.state.map.origin;
    let mut d = String::new();
    for i in 0..count.min(1 << 14) {
        let at = 16 + 32 + i * 16;
        let corner = |o: usize| -> Option<(f64, f64)> {
            Some((
                f64::from(r.i32(at + o)?) - origin.0,
                f64::from(r.i32(at + o + 4)?) - origin.1,
            ))
        };
        let ((left, top), (right, bottom)) = (corner(0)?, corner(8)?);
        let _ = write!(
            d,
            "M{} {} L{} {} L{} {} L{} {} Z ",
            num(left),
            num(top),
            num(right),
            num(top),
            num(right),
            num(bottom),
            num(left),
            num(bottom)
        );
    }
    canvas.clip(d, mode == 1);
    Some(())
}
