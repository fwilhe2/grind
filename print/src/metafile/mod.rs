// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Windows metafiles — WMF and EMF, the pictures old Office documents are full of — read as
//! layers in drawing order: SVGs of their lines, shapes and text, and the bitmaps between them,
//! which both backends draw as they draw any other picture (`doc/metafile-format.md`). What is
//! not read — raster operations other than a copy and an icon's mask, EMF+ records, regions
//! made by hand — is left out, never guessed at.

use std::fmt::Write as _;

use crate::fonts::Fonts;

mod dib;
mod emf;
mod wmf;

use dib::{Dib, Source, clipped, dib, masked};

/// A metafile as drawn: its picture's size in its own units, and its layers, first drawn first.
#[derive(Clone, Debug, PartialEq)]
pub struct Metafile {
    pub width: f64,
    pub height: f64,
    pub layers: Vec<Layer>,
}

/// An SVG the picture's size, or a bitmap.
#[derive(Clone, Debug, PartialEq)]
pub enum Layer {
    Svg(Vec<u8>),
    Bitmap(Bitmap),
}

/// A bitmap a metafile holds, where it is drawn: `x`, `y`, `width` and `height` in the
/// metafile's units, and the picture's bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub mime: &'static str,
    pub data: Vec<u8>,
}

/// Whether `mime` names a Windows metafile.
#[must_use]
pub(crate) fn is_metafile(mime: &str) -> bool {
    matches!(
        mime,
        "image/x-wmf" | "image/wmf" | "image/x-emf" | "image/emf"
    )
}

/// The metafile in `data`, WMF or EMF by its bytes, its text set in the faces of `fonts` that
/// its fonts resolve to, or `None` for anything else or one that draws nothing.
#[must_use]
pub fn read(data: &[u8], fonts: &Fonts) -> Option<Metafile> {
    if is_emf(data) {
        emf::emf(data, fonts)
    } else {
        wmf::wmf(data, fonts)
    }
}

fn is_emf(data: &[u8]) -> bool {
    data.get(..4) == Some(&[1, 0, 0, 0]) && data.get(40..44) == Some(b" EMF")
}

/// The font faces a metafile's text names and the characters it shows, for the fonts a document
/// is set in to include.
#[must_use]
pub fn wants(data: &[u8]) -> (Vec<String>, Vec<char>) {
    let b = Bytes(data);
    let mut faces: Vec<String> = Vec::new();
    let mut chars: Vec<char> = Vec::new();
    let mut face = |face: String| {
        if !face.is_empty() && !faces.contains(&face) && faces.len() < 64 {
            faces.push(face);
        }
    };
    let mut text = |text: String| {
        for c in text.chars() {
            if !c.is_whitespace() && !chars.contains(&c) && chars.len() < 4096 {
                chars.push(c);
            }
        }
    };
    if is_emf(data) {
        let mut at = 0usize;
        while let (Some(kind), Some(size)) = (b.u32(at), b.u32(at + 4)) {
            let size = size as usize;
            if size < 8 || kind == 14 || at + size > data.len() {
                break;
            }
            let r = Bytes(&data[at..at + size]);
            match kind {
                82 => face(utf16(r.0.get(40..104).unwrap_or_default())),
                83 | 84 => {
                    let (count, off) = (
                        r.u32(44).unwrap_or(0) as usize,
                        r.u32(48).unwrap_or(0) as usize,
                    );
                    let width = if kind == 84 { 2 } else { 1 };
                    if let Some(bytes) = r.0.get(off..off + count.min(1 << 16) * width) {
                        text(if kind == 84 {
                            utf16(bytes)
                        } else {
                            latin1(bytes)
                        });
                    }
                }
                _ => {}
            }
            at += size;
        }
    } else {
        let mut at = if b.u32(0) == Some(0x9AC6_CDD7) { 22 } else { 0 };
        at += usize::from(b.u16(at + 2).unwrap_or(9)) * 2;
        while let (Some(size), Some(function)) = (b.u32(at), b.u16(at + 4)) {
            let size = size as usize * 2;
            if size < 6 || function == 0 || at + size > data.len() {
                break;
            }
            let r = Bytes(&data[at + 6..at + size]);
            match function {
                0x02FB => face(latin1(r.0.get(18..50).unwrap_or_default())),
                0x0521 => {
                    let count = usize::from(r.u16(0).unwrap_or(0));
                    text(latin1(r.0.get(2..2 + count).unwrap_or_default()));
                }
                0x0A32 => {
                    let count = usize::from(r.u16(4).unwrap_or(0));
                    let offset = if r.u16(6).unwrap_or(0) & 0x6 != 0 {
                        16
                    } else {
                        8
                    };
                    text(latin1(r.0.get(offset..offset + count).unwrap_or_default()));
                }
                _ => {}
            }
            at += size;
        }
    }
    (faces, chars)
}

/// Little-endian readers that answer `None` past the end.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.0.get(at..at + 2)?.try_into().ok()?))
    }
    fn i16(&self, at: usize) -> Option<i16> {
        Some(i16::from_le_bytes(self.0.get(at..at + 2)?.try_into().ok()?))
    }
    fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.0.get(at..at + 4)?.try_into().ok()?))
    }
    fn i32(&self, at: usize) -> Option<i32> {
        Some(i32::from_le_bytes(self.0.get(at..at + 4)?.try_into().ok()?))
    }
    fn f32(&self, at: usize) -> Option<f32> {
        Some(f32::from_le_bytes(self.0.get(at..at + 4)?.try_into().ok()?))
    }
    /// A `COLORREF`: `0x00BBGGRR`.
    fn color(&self, at: usize) -> Option<Rgb> {
        let value = self.u32(at)?;
        Some(Rgb(value as u8, (value >> 8) as u8, (value >> 16) as u8))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

impl Rgb {
    const BLACK: Rgb = Rgb(0, 0, 0);
    const WHITE: Rgb = Rgb(255, 255, 255);

    fn css(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// A pen: no stroke, or one this wide in the metafile's logical units (zero: one device unit)
/// in this colour, dashed by its style (`PS_DASH` … `PS_DASHDOTDOT`, 1–4; 0 solid).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pen {
    None,
    Solid { width: f64, color: Rgb, dash: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Brush {
    None,
    Solid(Rgb),
}

#[derive(Clone, Debug, PartialEq)]
struct Font {
    /// `lfHeight`, logical units: negative is the em, positive the cell.
    height: f64,
    escapement: f64,
    weight: i32,
    italic: bool,
    underline: bool,
    face: String,
    /// `lfPitchAndFamily`'s family: roman, swiss, modern.
    family: u8,
}

impl Default for Font {
    fn default() -> Self {
        Font {
            height: -12.0,
            escapement: 0.0,
            weight: 400,
            italic: false,
            underline: false,
            face: String::new(),
            family: 0x20,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(Font),
    /// A palette, region or anything else selected without effect here.
    Other,
}

/// The device context's state that `SaveDC` keeps.
#[derive(Clone, Debug, PartialEq)]
struct State {
    pen: Pen,
    brush: Brush,
    font: Font,
    text: Rgb,
    background: Rgb,
    opaque: bool,
    evenodd: bool,
    align: u32,
    at: (f64, f64),
    map: Map,
    /// The clip in force, by its place in [`Canvas::clips`].
    clip: Option<usize>,
}

impl Default for State {
    fn default() -> Self {
        State {
            pen: Pen::Solid {
                width: 0.0,
                color: Rgb::BLACK,
                dash: 0,
            },
            brush: Brush::Solid(Rgb::WHITE),
            font: Font::default(),
            text: Rgb::BLACK,
            background: Rgb::WHITE,
            opaque: true,
            evenodd: true,
            align: 0,
            at: (0.0, 0.0),
            map: Map::default(),
            clip: None,
        }
    }
}

/// Logical units to the picture's: the world transform, the window and viewport, the mapping
/// mode, and where the picture's frame starts.
#[derive(Clone, Debug, PartialEq)]
struct Map {
    world: [f64; 6],
    mode: u32,
    window: ((f64, f64), (f64, f64)),
    viewport: ((f64, f64), (f64, f64)),
    /// Device units per millimetre, for the metric and English modes.
    per_mm: (f64, f64),
    /// The frame's top-left corner in device units, subtracted last.
    origin: (f64, f64),
    /// WMF: device units are the frame's, scaled from the window.
    wmf: Option<(f64, f64)>,
}

impl Default for Map {
    fn default() -> Self {
        Map {
            world: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            mode: 1,
            window: ((0.0, 0.0), (1.0, 1.0)),
            viewport: ((0.0, 0.0), (1.0, 1.0)),
            per_mm: (1.0, 1.0),
            origin: (0.0, 0.0),
            wmf: None,
        }
    }
}

impl Map {
    /// How far a logical unit goes across and down, page to device, by the mapping mode.
    fn scale(&self) -> (f64, f64) {
        if let Some((width, height)) = self.wmf {
            let (_, (ex, ey)) = self.window;
            return (width / nonzero(ex), height / nonzero(ey));
        }
        let ((_, (wx, wy)), (_, (vx, vy))) = (self.window, self.viewport);
        let metric = |mm: f64| (self.per_mm.0 * mm, -self.per_mm.1 * mm);
        match self.mode {
            // MM_LOMETRIC, MM_HIMETRIC, MM_LOENGLISH, MM_HIENGLISH, MM_TWIPS.
            2 => metric(0.1),
            3 => metric(0.01),
            4 => metric(0.254),
            5 => metric(0.0254),
            6 => metric(25.4 / 1440.0),
            // MM_ISOTROPIC: one scale both ways, the smaller, each keeping its sign.
            7 => {
                let (sx, sy) = (vx / nonzero(wx), vy / nonzero(wy));
                let both = sx.abs().min(sy.abs());
                (both.copysign(sx), both.copysign(sy))
            }
            8 => (vx / nonzero(wx), vy / nonzero(wy)),
            _ => (1.0, 1.0),
        }
    }

    /// A logical point in the picture's units.
    fn point(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let [m11, m12, m21, m22, dx, dy] = self.world;
        let (px, py) = (x * m11 + y * m21 + dx, x * m12 + y * m22 + dy);
        let ((wox, woy), _) = self.window;
        let ((vox, voy), _) = self.viewport;
        let (sx, sy) = self.scale();
        let vo = if self.wmf.is_some() {
            (0.0, 0.0)
        } else {
            (vox, voy)
        };
        (
            (px - wox) * sx + vo.0 - self.origin.0,
            (py - woy) * sy + vo.1 - self.origin.1,
        )
    }

    /// How long a logical length is in the picture's units, across and down.
    fn length(&self, length: f64) -> (f64, f64) {
        let [m11, m12, m21, m22, ..] = self.world;
        let (sx, sy) = self.scale();
        (
            length * m11.hypot(m12) * sx.abs(),
            length * m21.hypot(m22) * sy.abs(),
        )
    }
}

fn nonzero(value: f64) -> f64 {
    if value == 0.0 { 1.0 } else { value }
}

/// What both formats draw with: the state, the objects, the SVG so far and the bitmaps.
struct Canvas<'f> {
    fonts: &'f Fonts,
    state: State,
    saved: Vec<State>,
    objects: Vec<Option<Object>>,
    /// The SVG layer being drawn, and the layers before it.
    svg: String,
    layers: Vec<Layer>,
    /// Every clip made: a path in the picture's units, the clip it is within, and whether it is
    /// rectangles cut out of everything (`ExcludeClipRect`), filled even-odd.
    clips: Vec<(String, Option<usize>, bool)>,
    /// The path being recorded between `BeginPath` and `EndPath`, as SVG path data.
    path: Option<String>,
    /// The picture's units per device unit, for a pen of width zero.
    hairline: f64,
    /// A bitmap drawn `SRCAND` — a mask — and its rectangle, for the bitmap after it.
    mask: Option<([f64; 4], tiny_skia::Pixmap)>,
}

impl<'f> Canvas<'f> {
    fn new(fonts: &'f Fonts) -> Self {
        Canvas {
            fonts,
            state: State::default(),
            saved: Vec::new(),
            objects: Vec::new(),
            svg: String::new(),
            layers: Vec::new(),
            clips: Vec::new(),
            path: None,
            hairline: 1.0,
            mask: None,
        }
    }

    fn select(&mut self, object: &Object) {
        match object {
            Object::Pen(pen) => self.state.pen = *pen,
            Object::Brush(brush) => self.state.brush = *brush,
            Object::Font(font) => self.state.font = font.clone(),
            Object::Other => {}
        }
    }

    /// A stock object (`0x80000000 | n`).
    fn stock(&mut self, n: u32) {
        let object = match n {
            0 => Object::Brush(Brush::Solid(Rgb::WHITE)),
            1 => Object::Brush(Brush::Solid(Rgb(192, 192, 192))),
            2 => Object::Brush(Brush::Solid(Rgb(128, 128, 128))),
            3 => Object::Brush(Brush::Solid(Rgb(64, 64, 64))),
            4 => Object::Brush(Brush::Solid(Rgb::BLACK)),
            5 => Object::Brush(Brush::None),
            6 => Object::Pen(Pen::Solid {
                width: 0.0,
                color: Rgb::WHITE,
                dash: 0,
            }),
            7 => Object::Pen(Pen::Solid {
                width: 0.0,
                color: Rgb::BLACK,
                dash: 0,
            }),
            8 => Object::Pen(Pen::None),
            10..=17 => Object::Font(Font::default()),
            _ => Object::Other,
        };
        self.select(&object);
    }

    fn save(&mut self) {
        if self.saved.len() < 256 {
            self.saved.push(self.state.clone());
        }
    }

    /// `RestoreDC`: `-n` goes back `n` saves.
    fn restore(&mut self, relative: i32) {
        let back = relative.unsigned_abs().max(1) as usize;
        for _ in 1..back {
            self.saved.pop();
        }
        if let Some(state) = self.saved.pop() {
            self.state = state;
        }
    }

    /// The pen's SVG attributes.
    fn stroke(&self) -> String {
        match self.state.pen {
            Pen::None => " stroke=\"none\"".to_owned(),
            Pen::Solid { width, color, dash } => {
                let (wx, wy) = self.state.map.length(width);
                let width = f64::midpoint(wx, wy).max(self.hairline);
                // Windows' own lengths for a dashed cosmetic pen, as many widths of a wider one.
                let pattern: &[f64] = match dash {
                    1 => &[18.0, 6.0],
                    2 => &[3.0, 3.0],
                    3 => &[9.0, 6.0, 3.0, 6.0],
                    4 => &[9.0, 3.0, 3.0, 3.0, 3.0, 3.0],
                    _ => &[],
                };
                let unit = if width > self.hairline {
                    width / 3.0
                } else {
                    self.hairline
                };
                let dashes: Vec<String> = pattern.iter().map(|d| num(d * unit)).collect();
                let dashes = if dashes.is_empty() {
                    String::new()
                } else {
                    format!(" stroke-dasharray=\"{}\"", dashes.join(" "))
                };
                format!(
                    " stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"{}\"{dashes}",
                    color.css(),
                    num(width),
                    if dashes.is_empty() { "round" } else { "butt" }
                )
            }
        }
    }

    fn fill(&self) -> String {
        match self.state.brush {
            Brush::None => " fill=\"none\"".to_owned(),
            Brush::Solid(color) => format!(
                " fill=\"{}\" fill-rule=\"{}\"",
                color.css(),
                if self.state.evenodd {
                    "evenodd"
                } else {
                    "nonzero"
                }
            ),
        }
    }

    /// Path data through logical points, closed when `closed`.
    fn data(&self, points: &[(f64, f64)], closed: bool) -> String {
        let mut d = String::new();
        for (at, &point) in points.iter().enumerate() {
            let (x, y) = self.state.map.point(point);
            let _ = write!(
                d,
                "{}{} {} ",
                if at == 0 { 'M' } else { 'L' },
                num(x),
                num(y)
            );
        }
        if closed && !points.is_empty() {
            d.push('Z');
        }
        d
    }

    /// Draw — or, inside a path, record — shapes from path data: filled when `filled`, and
    /// stroked with the pen.
    fn shape(&mut self, d: &str, filled: bool) {
        if d.is_empty() {
            return;
        }
        if let Some(path) = &mut self.path {
            path.push_str(d);
            return;
        }
        let fill = if filled {
            self.fill()
        } else {
            " fill=\"none\"".to_owned()
        };
        let element = format!("<path d=\"{d}\"{fill}{}/>", self.stroke());
        self.put(&element);
    }

    fn polygon(&mut self, points: &[(f64, f64)]) {
        let d = self.data(points, true);
        self.shape(&d, true);
    }

    fn polyline(&mut self, points: &[(f64, f64)]) {
        let d = self.data(points, false);
        self.shape(&d, false);
    }

    /// Cubic Béziers from the current position or the first point: each three points after
    /// that one segment.
    fn beziers(&mut self, points: &[(f64, f64)], from_current: bool) {
        let mut all: Vec<(f64, f64)> = Vec::with_capacity(points.len() + 1);
        if from_current {
            all.push(self.state.at);
        }
        all.extend_from_slice(points);
        let Some(&first) = all.first() else {
            return;
        };
        let (x, y) = self.state.map.point(first);
        let mut d = if from_current && self.path.is_some() {
            String::new()
        } else {
            format!("M{} {} ", num(x), num(y))
        };
        for chunk in all[1..].as_chunks::<3>().0 {
            d.push('C');
            for &point in chunk {
                let (x, y) = self.state.map.point(point);
                let _ = write!(d, "{} {} ", num(x), num(y));
            }
        }
        if let Some(&last) = all.last() {
            self.state.at = last;
        }
        self.shape(&d, false);
    }

    fn rectangle(&mut self, (left, top): (f64, f64), (right, bottom): (f64, f64)) {
        self.polygon(&[(left, top), (right, top), (right, bottom), (left, bottom)]);
    }

    fn ellipse(&mut self, (left, top): (f64, f64), (right, bottom): (f64, f64)) {
        let (cx, cy) = (f64::midpoint(left, right), f64::midpoint(top, bottom));
        let (rx, ry) = ((right - left).abs() / 2.0, (bottom - top).abs() / 2.0);
        // Four arcs as Béziers, which survive any of the transforms.
        const K: f64 = 0.552_284_75;
        let p = |x: f64, y: f64| self.state.map.point((x, y));
        let points = [
            p(cx + rx, cy),
            p(cx + rx, cy + ry * K),
            p(cx + rx * K, cy + ry),
            p(cx, cy + ry),
            p(cx - rx * K, cy + ry),
            p(cx - rx, cy + ry * K),
            p(cx - rx, cy),
            p(cx - rx, cy - ry * K),
            p(cx - rx * K, cy - ry),
            p(cx, cy - ry),
            p(cx + rx * K, cy - ry),
            p(cx + rx, cy - ry * K),
            p(cx + rx, cy),
        ];
        let mut d = format!("M{} {} ", num(points[0].0), num(points[0].1));
        for chunk in points[1..].as_chunks::<3>().0 {
            d.push('C');
            for (x, y) in chunk {
                let _ = write!(d, "{} {} ", num(*x), num(*y));
            }
        }
        d.push('Z');
        self.shape(&d, true);
    }

    fn move_to(&mut self, point: (f64, f64)) {
        self.state.at = point;
        if let Some(path) = &mut self.path {
            let (x, y) = self.state.map.point(point);
            let _ = write!(path, "M{} {} ", num(x), num(y));
        }
    }

    fn line_to(&mut self, point: (f64, f64)) {
        let from = self.state.at;
        self.state.at = point;
        if let Some(path) = &mut self.path {
            let (x, y) = self.state.map.point(point);
            let _ = write!(path, "L{} {} ", num(x), num(y));
            return;
        }
        self.polyline(&[from, point]);
    }

    /// `FillPath`, `StrokePath`, `StrokeAndFillPath`: draw the path recorded.
    fn draw_path(&mut self, fill: bool, stroke: bool) {
        let Some(d) = self.path.take() else {
            return;
        };
        let fill = if fill {
            self.fill()
        } else {
            " fill=\"none\"".to_owned()
        };
        let stroke = if stroke {
            self.stroke()
        } else {
            " stroke=\"none\"".to_owned()
        };
        self.put(&format!("<path d=\"{d}\"{fill}{stroke}/>"));
    }

    /// A rectangle filled with `color`, corners in logical units.
    fn block(&mut self, (left, top): (f64, f64), (right, bottom): (f64, f64), color: Rgb) {
        let d = self.data(
            &[(left, top), (right, top), (right, bottom), (left, bottom)],
            true,
        );
        self.put(&format!("<path d=\"{d}\" fill=\"{}\"/>", color.css()));
    }

    /// Text at a logical point, by `SetTextAlign`'s rules, its characters at `dx`'s logical
    /// advances when given.
    fn text(&mut self, at: (f64, f64), text: &str, dx: Option<&[f64]>) {
        if text.trim().is_empty() || self.path.is_some() {
            return;
        }
        let align = self.state.align;
        let at = if align & 1 != 0 { self.state.at } else { at };
        let font = self.state.font.clone();
        let map = self.state.map.clone();
        let (family, ascent, descent) = face_of(self.fonts, &font);
        // The em: `lfHeight` negative is it, positive the cell — the face's ascent and descent.
        let em_logical = if font.height < 0.0 {
            -font.height
        } else if font.height > 0.0 {
            font.height / (ascent + descent).max(0.1)
        } else {
            12.0
        };
        let em = map.length(em_logical).1.max(0.01);
        let (x, y) = map.point(at);
        let angle = -font.escapement / 10.0;
        let (sin, cos) = angle.to_radians().sin_cos();
        // Down from the reference point to the baseline, by the face's own ascent and descent,
        // and back to where the text starts.
        let drop = match align & 0x18 {
            0x18 => 0.0,
            0x08 => -descent * em,
            _ => ascent * em,
        };
        let advances: Option<Vec<f64>> = dx.map(|dx| dx.iter().map(|d| map.length(*d).0).collect());
        let total: Option<f64> = advances.as_ref().map(|a| a.iter().sum());
        let shift = match (align & 6, total) {
            (6, Some(total)) => -total / 2.0,
            (2, Some(total)) => -total,
            _ => 0.0,
        };
        let anchor = match (align & 6, total) {
            (6, None) => " text-anchor=\"middle\"",
            (2, None) => " text-anchor=\"end\"",
            _ => "",
        };
        let base = (x + shift * cos - drop * sin, y + shift * sin + drop * cos);
        let family = format!("'{}'", escape(&family.replace('\'', "")));
        let weight = if font.weight >= 600 { "bold" } else { "normal" };
        let style = if font.italic { "italic" } else { "normal" };
        let decoration = if font.underline {
            " text-decoration=\"underline\""
        } else {
            ""
        };
        let rotate = if angle == 0.0 {
            String::new()
        } else {
            format!(
                " transform=\"rotate({} {} {})\"",
                num(angle),
                num(base.0),
                num(base.1)
            )
        };
        // With advances, each character at its own x, unrotated, the rotation applied over all —
        // but for a right-to-left script only where it starts, so its letters are still shaped
        // and joined, and ordered right to left.
        let rtl = text
            .chars()
            .any(|c| matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF));
        let xs = match &advances {
            Some(advances) if !rtl => {
                let mut x = base.0;
                let mut out = String::new();
                for (at, (_, advance)) in text.chars().zip(advances).enumerate() {
                    if at > 0 {
                        out.push(' ');
                    }
                    out.push_str(&num(x));
                    x += advance;
                }
                out
            }
            _ => num(base.0),
        };
        let element = format!(
            "<text x=\"{xs}\" y=\"{}\" font-family=\"{family}\" font-size=\"{}\" font-weight=\"{weight}\" font-style=\"{style}\" fill=\"{}\"{anchor}{decoration}{rotate} xml:space=\"preserve\">{}</text>",
            num(base.1),
            num(em),
            self.state.text.css(),
            escape(text)
        );
        self.put(&element);
    }

    /// Add an element to the SVG layer, inside the clip in force.
    fn put(&mut self, element: &str) {
        match self.state.clip {
            Some(clip) => {
                let _ = writeln!(self.svg, "<g clip-path=\"url(#c{clip})\">{element}</g>");
            }
            None => {
                let _ = writeln!(self.svg, "{element}");
            }
        }
    }

    /// Clip to path data in the picture's units: within the clip in force when `within`, else
    /// instead of it.
    fn clip(&mut self, d: String, within: bool) {
        if self.clips.len() >= 4096 {
            return;
        }
        let parent = if within { self.state.clip } else { None };
        self.clips.push((d, parent, false));
        self.state.clip = Some(self.clips.len() - 1);
    }

    /// `ExcludeClipRect`: cut a logical rectangle out of the clip. Cuts in a row share one clip,
    /// everything with the rectangles as holes.
    fn exclude(&mut self, (left, top): (f64, f64), (right, bottom): (f64, f64)) {
        let hole = self.data(
            &[(left, top), (right, top), (right, bottom), (left, bottom)],
            true,
        );
        if let Some(index) = self.state.clip
            && let Some((d, _, true)) = self.clips.get_mut(index)
        {
            d.push_str(&hole);
            return;
        }
        if self.clips.len() >= 4096 {
            return;
        }
        let everything =
            "M-10000000 -10000000 L10000000 -10000000 L10000000 10000000 L-10000000 10000000 Z";
        self.clips
            .push((format!("{everything} {hole}"), self.state.clip, true));
        self.state.clip = Some(self.clips.len() - 1);
    }

    /// `IntersectClipRect`: clip to a logical rectangle too.
    fn clip_rect(&mut self, (left, top): (f64, f64), (right, bottom): (f64, f64)) {
        let d = self.data(
            &[(left, top), (right, top), (right, bottom), (left, bottom)],
            true,
        );
        self.clip(d, true);
    }

    /// The clip in force and those it is within, innermost first, each with whether it is
    /// filled even-odd.
    fn clip_chain(&self) -> Vec<(&str, bool)> {
        let mut out = Vec::new();
        let mut at = self.state.clip;
        while let Some(index) = at {
            let Some((d, parent, evenodd)) = self.clips.get(index) else {
                break;
            };
            out.push((d.as_str(), *evenodd));
            at = *parent;
            if out.len() > 64 {
                break;
            }
        }
        out
    }

    /// A DIB copied into a logical rectangle by the raster operation `rop`: `SRCCOPY` draws it;
    /// `SRCAND` keeps it as a mask, and a `SRCINVERT` or `SRCPAINT` into the same rectangle after
    /// it draws its bitmap where the mask is black — what the pair does on white paper, the way
    /// an icon is drawn with its own transparency. A `SRCINVERT` with no mask is drawn as a copy;
    /// any other operation is left out.
    fn blit(
        &mut self,
        (rop, source): (u32, Option<Source>),
        info: &[u8],
        bits: Option<&[u8]>,
        corners: [(f64, f64); 2],
    ) {
        let dib = |info: &[u8], bits: Option<&[u8]>| dib(info, bits).map(|d| d.cut(source));
        const SRCCOPY: u32 = 0x00CC_0020;
        const SRCAND: u32 = 0x0088_00C6;
        const SRCINVERT: u32 = 0x0066_0046;
        const SRCPAINT: u32 = 0x00EE_0086;
        let (a, b) = (
            self.state.map.point(corners[0]),
            self.state.map.point(corners[1]),
        );
        let rect = [
            a.0.min(b.0),
            a.1.min(b.1),
            (a.0 - b.0).abs(),
            (a.1 - b.1).abs(),
        ];
        if rect[2] <= 0.0 || rect[3] <= 0.0 {
            return;
        }
        let mask = self.mask.take();
        let picture = match rop {
            SRCCOPY => dib(info, bits).and_then(Dib::encoded),
            SRCAND => {
                if let Some(Dib::Pixels(pixels)) = dib(info, bits) {
                    self.mask = Some((rect, pixels));
                }
                return;
            }
            SRCINVERT | SRCPAINT => match (dib(info, bits), mask) {
                (Some(Dib::Pixels(mut pixels)), Some((at, mask)))
                    if at.iter().zip(rect).all(|(a, b)| (a - b).abs() < 0.5) =>
                {
                    masked(&mut pixels, &mask);
                    Dib::Pixels(pixels).encoded()
                }
                (Some(picture), _) if rop == SRCINVERT => picture.encoded(),
                _ => None,
            },
            _ => None,
        };
        let Some((mime, data)) = picture else {
            return;
        };
        let data = match (self.clip_chain(), mime) {
            (chain, "image/png") if !chain.is_empty() => {
                clipped(&data, rect, &chain).unwrap_or(data)
            }
            _ => data,
        };
        if !self.svg.is_empty() {
            let body = std::mem::take(&mut self.svg);
            self.layers.push(Layer::Svg(body.into_bytes()));
        }
        self.layers.push(Layer::Bitmap(Bitmap {
            x: rect[0],
            y: rect[1],
            width: rect[2],
            height: rect[3],
            mime,
            data,
        }));
    }

    /// A raster operation with no source: `PATCOPY` fills with the brush, `BLACKNESS` and
    /// `WHITENESS` with black and white; any other is left out.
    fn pattern(&mut self, rop: u32, corners: [(f64, f64); 2]) {
        let color = match rop {
            0x00F0_0021 => match self.state.brush {
                Brush::Solid(color) => color,
                Brush::None => return,
            },
            0x0000_0042 => Rgb::BLACK,
            0x00FF_0062 => Rgb::WHITE,
            _ => return,
        };
        self.block(corners[0], corners[1], color);
    }

    fn finish(mut self, width: f64, height: f64) -> Option<Metafile> {
        if !self.svg.is_empty() {
            let body = std::mem::take(&mut self.svg);
            self.layers.push(Layer::Svg(body.into_bytes()));
        }
        if self.layers.is_empty()
            || !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        {
            return None;
        }
        // Each SVG layer a document of its own, every clip declared in each.
        let mut defs = String::new();
        for (index, (d, parent, evenodd)) in self.clips.iter().enumerate() {
            let within = parent.map_or_else(String::new, |p| format!(" clip-path=\"url(#c{p})\""));
            let rule = if *evenodd {
                " clip-rule=\"evenodd\""
            } else {
                ""
            };
            let _ = writeln!(
                defs,
                "<clipPath id=\"c{index}\"{within}><path d=\"{d}\"{rule}/></clipPath>"
            );
        }
        let layers = self
            .layers
            .into_iter()
            .map(|layer| match layer {
                Layer::Svg(body) => Layer::Svg(
                    format!(
                        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" preserveAspectRatio=\"none\">\n<defs>{defs}</defs>\n{}</svg>\n",
                        String::from_utf8_lossy(&body),
                        w = num(width),
                        h = num(height)
                    )
                    .into_bytes(),
                ),
                bitmap @ Layer::Bitmap(_) => bitmap,
            })
            .collect();
        Some(Metafile {
            width,
            height,
            layers,
        })
    }
}

/// The face a font's text is set in — the family of `fonts` its name resolves to, through a
/// metric-compatible twin or its kind (`lfPitchAndFamily`) — and that face's `hhea` ascent and
/// descent in ems, by which LibreOffice places a metafile's text (`doc/metafile-format.md`).
fn face_of(fonts: &Fonts, font: &Font) -> (String, f64, f64) {
    use skrifa::raw::TableProvider;
    let generic = match font.family & 0xF0 {
        0x10 => "roman",
        0x30 => "modern",
        _ => "swiss",
    };
    let name = Some(font.face.as_str()).filter(|face| !face.is_empty());
    let resolved = fonts.resolve_hinted(name, font.weight >= 600, font.italic, Some(generic));
    let face = fonts.face(resolved.face);
    let metrics = skrifa::FontRef::from_index(face.bytes(), face.index)
        .ok()
        .and_then(|font| {
            let hhea = font.hhea().ok()?;
            let em = f64::from(font.head().ok()?.units_per_em().max(1));
            Some((
                f64::from(hhea.ascender().to_i16()) / em,
                -f64::from(hhea.descender().to_i16()) / em,
            ))
        })
        .unwrap_or((0.905, 0.212));
    (face.family.clone(), metrics.0, metrics.1)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

fn num(value: f64) -> String {
    let value = if value.is_finite() { value } else { 0.0 };
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// A pen from its style (`PS_NULL` is 5, 1–4 the dashes), logical width and colour.
fn pen(style: u32, width: f64, color: Rgb) -> Pen {
    match style & 0x0F {
        5 => Pen::None,
        dash @ 1..=4 => Pen::Solid { width, color, dash },
        _ => Pen::Solid {
            width,
            color,
            dash: 0,
        },
    }
}

/// A brush from its style: `BS_NULL` (1) none, any other this colour.
fn brush(style: u32, color: Rgb) -> Brush {
    if style == 1 {
        Brush::None
    } else {
        Brush::Solid(color)
    }
}

/// UTF-16 to its first NUL; half a surrogate pair — one a record split from its other half — is
/// nothing, as LibreOffice draws it.
fn utf16(bytes: &[u8]) -> String {
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0);
    char::decode_utf16(units).filter_map(Result::ok).collect()
}

fn latin1(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| char::from(*b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A placeable WMF of `records` (each its function and parameter words) over a 0..1000
    /// square at 1000 units to the inch.
    fn wmf(records: &[(u16, Vec<u16>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(0x9AC6_CDD7u32.to_le_bytes());
        out.extend([0, 0]);
        for value in [0i16, 0, 1000, 1000] {
            out.extend(value.to_le_bytes());
        }
        out.extend(1000u16.to_le_bytes());
        out.extend([0, 0, 0, 0, 0, 0]);
        let body: Vec<u8> = records
            .iter()
            .flat_map(|(function, words)| {
                let mut record = Vec::new();
                record.extend((3 + words.len() as u32).to_le_bytes());
                record.extend(function.to_le_bytes());
                for word in words {
                    record.extend(word.to_le_bytes());
                }
                record
            })
            .chain([3, 0, 0, 0, 0, 0])
            .collect();
        out.extend([1, 0, 9, 0, 0, 3]);
        out.extend(((9 + body.len() / 2) as u32).to_le_bytes());
        out.extend([0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend(body);
        out
    }

    /// Every SVG layer's text, in order.
    fn svg_of(picture: &Metafile) -> String {
        picture
            .layers
            .iter()
            .filter_map(|layer| match layer {
                Layer::Svg(svg) => Some(String::from_utf8_lossy(svg).into_owned()),
                Layer::Bitmap(_) => None,
            })
            .collect()
    }

    /// A white fill, a bitmap, then a line: three layers in that order, the line clipped to the
    /// rectangle `IntersectClipRect` set, and the bitmap transparent outside it.
    #[test]
    fn layers_keep_the_drawing_order_and_the_clip() {
        // A 2 × 2 DIB at 24 bits, all red.
        let mut dib: Vec<u16> = vec![
            40, 0, 2, 0, 2, 0, 1, 24, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        // Each row two pixels of blue, green, red bytes 0, 0, 255, padded to eight bytes.
        dib.extend([0x0000, 0x00FF, 0xFF00, 0x0000].repeat(2));
        // META_DIBSTRETCHBLT: the operation, the source's size and corner, the destination's.
        let mut blit = vec![0x0020, 0x00CC, 2, 2, 0, 0, 1000, 1000, 0, 0];
        blit.extend(&dib);
        let data = wmf(&[
            (0x0416, vec![500, 500, 0, 0]),
            (0x02FC, vec![0, 0xFFFF, 0x00FF, 0]),
            (0x012D, vec![0]),
            (0x061D, vec![0x0021, 0x00F0, 1000, 1000, 0, 0]),
            (0x0B41, blit),
            (0x0214, vec![0, 0]),
            (0x0213, vec![1000, 1000]),
        ]);
        let picture = read(&data, &Fonts::bundled()).expect("a picture");
        let kinds: Vec<&str> = picture
            .layers
            .iter()
            .map(|layer| match layer {
                Layer::Svg(_) => "svg",
                Layer::Bitmap(_) => "bitmap",
            })
            .collect();
        assert_eq!(kinds, ["svg", "bitmap", "svg"]);
        let svg = svg_of(&picture);
        assert!(
            svg.contains("<clipPath id=\"c0\"><path d=\"M0 0 L500 0 L500 500 L0 500 Z\"/>"),
            "{svg}"
        );
        assert!(
            svg.contains("<g clip-path=\"url(#c0)\"><path d=\"M0 0 L1000 1000"),
            "{svg}"
        );
        let Layer::Bitmap(bitmap) = &picture.layers[1] else {
            unreachable!()
        };
        let pixels = tiny_skia::Pixmap::decode_png(&bitmap.data).unwrap();
        assert_eq!(pixels.pixel(0, 0).unwrap().alpha(), 255, "inside the clip");
        assert_eq!(pixels.pixel(1, 1).unwrap().alpha(), 0, "outside it");
    }

    /// An EMF of `records` (each its type and body after the type and size) on a device 1000
    /// pixels and 100 mm square, its frame the whole of it.
    fn emf(records: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut header = Vec::new();
        for value in [0i32, 0, 1000, 1000] {
            header.extend(value.to_le_bytes());
        }
        for value in [0i32, 0, 10_000, 10_000] {
            header.extend(value.to_le_bytes());
        }
        header.extend(b" EMF");
        header.resize(64, 0);
        for value in [1000i32, 1000, 100, 100] {
            header.extend(value.to_le_bytes());
        }
        let mut out = Vec::new();
        let all = std::iter::once((1u32, header))
            .chain(records.iter().cloned())
            .chain(std::iter::once((14u32, vec![0; 12])));
        for (kind, body) in all {
            out.extend(kind.to_le_bytes());
            out.extend((8 + body.len() as u32).to_le_bytes());
            out.extend(body);
        }
        out
    }

    fn le(values: &[i32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn an_emf_polygon_is_drawn_in_device_units_with_its_brush_and_pen() {
        let mut polygon = le(&[0, 0, 0, 0, 3]);
        for (x, y) in [(100i16, 100i16), (900, 100), (500, 800)] {
            polygon.extend(x.to_le_bytes());
            polygon.extend(y.to_le_bytes());
        }
        let data = emf(&[
            // EMR_CREATEBRUSHINDIRECT: slot 1, solid blue.
            (39, le(&[1, 0, 0x00FF_0000, 0])),
            (37, le(&[1])),
            // EMR_CREATEPEN: slot 2, dashed, 10 units, red.
            (38, le(&[2, 1, 10, 0, 0x0000_00FF])),
            (37, le(&[2])),
            (86, polygon),
        ]);
        let picture = read(&data, &Fonts::bundled()).expect("a picture");
        assert_eq!((picture.width, picture.height), (1000.0, 1000.0));
        let svg = svg_of(&picture);
        assert!(svg.contains("d=\"M100 100 L900 100 L500 800 Z\""), "{svg}");
        assert!(svg.contains("fill=\"#0000ff\""), "{svg}");
        assert!(
            svg.contains("stroke=\"#ff0000\" stroke-width=\"10\""),
            "{svg}"
        );
        assert!(svg.contains("stroke-dasharray"), "{svg}");
    }

    #[test]
    fn an_emf_world_transform_moves_what_follows() {
        let mut transform: Vec<u8> = [2.0f32, 0.0, 0.0, 2.0, 100.0, 50.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        transform.extend(4u32.to_le_bytes());
        let data = emf(&[(36, transform), (27, le(&[10, 10])), (54, le(&[20, 30]))]);
        let svg = svg_of(&read(&data, &Fonts::bundled()).unwrap());
        assert!(svg.contains("M120 70 L140 110"), "{svg}");
    }

    #[test]
    fn emf_text_is_placed_by_its_advances_and_its_align() {
        // EMR_EXTCREATEFONTINDIRECTW: slot 1, an em of 100, Arial.
        let mut font = le(&[1, -100, 0, 0, 0, 400]);
        font.extend([0u8; 8]);
        font.extend("Arial".encode_utf16().flat_map(u16::to_le_bytes));
        font.resize(4 + 92, 0);
        // EMR_EXTTEXTOUTW: bounds, mode, scales, at (200, 300), two characters, their advances.
        let mut text = le(&[0, 0, 0, 0, 1, 0, 0, 200, 300, 2, 76, 0, 0, 0, 0, 0, 80]);
        text.extend("Hi".encode_utf16().flat_map(u16::to_le_bytes));
        text.extend(le(&[60, 30]));
        let data = emf(&[(82, font), (37, le(&[1])), (22, le(&[24])), (84, text)]);
        let svg = svg_of(&read(&data, &Fonts::bundled()).unwrap());
        assert!(
            svg.contains("<text x=\"200 260\" y=\"300\""),
            "on its baseline, each at its advance: {svg}"
        );
        assert!(svg.contains("font-size=\"100\""), "{svg}");
        assert!(svg.contains(">Hi</text>"), "{svg}");
    }

    /// A WMF on a page, its left half red: red in the left of its box and white in the right,
    /// in the preview and in the PDF a renderer reads back.
    #[test]
    fn a_metafile_is_drawn_in_its_box_in_the_preview_and_the_pdf() {
        let data = wmf(&[
            (0x02FC, vec![0, 0x00FF, 0x0000, 0]),
            (0x012D, vec![0]),
            (0x02FA, vec![5, 0, 0, 0, 0]),
            (0x012D, vec![1]),
            (0x041B, vec![1000, 500, 0, 0]),
        ]);
        let page = crate::ops::Page {
            width: 100.0,
            height: 50.0,
            ops: vec![crate::ops::Op::Image {
                x: 10.0,
                y: 10.0,
                width: 40.0,
                height: 30.0,
                mime: "image/x-wmf".into(),
                data: std::sync::Arc::new(data),
                crop: [0.0; 4],
                mark: crate::ops::Mark::Decoration,
            }],
            lines: Vec::new(),
            start: None,
            end: None,
        };
        let fonts = Fonts::bundled();
        let raster = crate::raster::render(&page, &fonts, 1.0);
        assert_eq!(raster.pixel(20, 25), [255, 0, 0, 255]);
        assert_eq!(raster.pixel(45, 25), [255, 255, 255, 255]);
        let doc = crate::ops::Document {
            pages: vec![page.into()],
            outline: vec![],
            structure: vec![],
            keys: vec![],
            cut: None,
        };
        let bytes = crate::pdf::write(&doc, &fonts, &crate::pdf::Metadata::default()).unwrap();
        let pdf = hayro::hayro_syntax::Pdf::new(bytes).unwrap();
        let theirs = hayro::render(
            &pdf.pages()[0],
            &hayro::RenderCache::new(),
            &hayro::hayro_interpret::InterpreterSettings::default(),
            &hayro::RenderSettings::default(),
            &hayro::PixmapSettings {
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                ..hayro::PixmapSettings::default()
            },
        );
        let at = |x: usize, y: usize| {
            let i = (y * theirs.width() as usize + x) * 4;
            theirs.data_as_u8_slice()[i..i + 3].to_vec()
        };
        assert_eq!(at(20, 25), vec![255, 0, 0]);
        assert_eq!(at(45, 25), vec![255, 255, 255]);
    }

    #[test]
    fn a_wmf_rectangle_is_filled_with_its_brush_in_the_bounds_units() {
        let data = wmf(&[
            (0x02FC, vec![0, 0x00FF, 0x0000, 0]),
            (0x012D, vec![0]),
            (0x041B, vec![600, 900, 100, 200]),
        ]);
        let picture = read(&data, &Fonts::bundled()).expect("a picture");
        assert_eq!((picture.width, picture.height), (1000.0, 1000.0));
        let svg = svg_of(&picture);
        assert!(
            svg.contains("M200 100 L900 100 L900 600 L200 600 Z"),
            "{svg}"
        );
        assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
    }

    #[test]
    fn a_wmf_window_scales_its_records_to_the_bounds() {
        let data = wmf(&[
            (0x020B, vec![0, 0]),
            (0x020C, vec![100, 100]),
            (0x0214, vec![0, 0]),
            (0x0213, vec![100, 50]),
        ]);
        let svg = svg_of(&read(&data, &Fonts::bundled()).unwrap());
        assert!(svg.contains("M0 0 L500 1000"), "{svg}");
    }

    #[test]
    fn a_dib_reads_bottom_up_with_its_palette() {
        // 2 × 2 at one bit: palette black, white; the bottom row first.
        let mut dib = Vec::new();
        dib.extend(40u32.to_le_bytes());
        dib.extend(2i32.to_le_bytes());
        dib.extend(2i32.to_le_bytes());
        dib.extend(1u16.to_le_bytes());
        dib.extend(1u16.to_le_bytes());
        dib.extend([0u8; 24]);
        dib.extend([0, 0, 0, 0, 255, 255, 255, 0]);
        dib.extend([0b0100_0000, 0, 0, 0, 0b1000_0000, 0, 0, 0]);
        let (mime, png) = super::dib(&dib, None)
            .and_then(Dib::encoded)
            .expect("a picture");
        assert_eq!(mime, "image/png");
        let pixmap = tiny_skia::Pixmap::decode_png(&png).unwrap();
        let at = |x: u32, y: u32| pixmap.pixel(x, y).unwrap().red();
        assert_eq!((at(0, 0), at(1, 0), at(0, 1), at(1, 1)), (255, 0, 0, 255));
    }

    #[test]
    fn numbers_are_short_and_never_negative_zero() {
        assert_eq!(num(12.5), "12.5");
        assert_eq!(num(3.0), "3");
        assert_eq!(num(-0.0001), "0");
    }
}
