// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A frame's pixels as a PNG — written here rather than by `NSBitmapImageRep`, so that two renders
//! of one document are the same **bytes** and not merely the same picture.
//!
//! Decision 9 asserts `--render-to` twice and compares the files, the way `ui_win32`'s BMP frames
//! are compared. A system encoder is free to write a timestamp, a colour profile it picked, or a
//! compression level that changes between releases; this one writes the four chunks a PNG needs
//! and nothing else, compressed by the `flate2` the workspace already links for `zip`. Portable,
//! so what it writes is checked on any host.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// The eight bytes every PNG starts with.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// `rgba` — `height` rows of `width` pixels, four bytes each, top row first — as a PNG.
///
/// Straight (not premultiplied) alpha, which is what a frame's opaque pixels are either way:
/// every frame starts by filling its whole rectangle.
///
/// # Panics
///
/// When `rgba` is not exactly `width * height * 4` bytes, which is a caller's bug rather than a
/// picture this could honestly write.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = width as usize * 4;
    assert_eq!(
        rgba.len(),
        stride * height as usize,
        "one RGBA pixel per place"
    );

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    // Eight bits a channel, colour type 6 (RGBA), deflate, the one filter method, no interlace.
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

    // Every row carries its filter byte; `0` is none, which keeps the bytes a function of the
    // pixels alone.
    let mut filtered = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact(stride.max(1)).take(height as usize) {
        filtered.push(0);
        filtered.extend_from_slice(row);
    }
    let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
    zlib.write_all(&filtered)
        .expect("writing into a Vec cannot fail");
    let idat = zlib.finish().expect("finishing into a Vec cannot fail");

    let mut out = SIGNATURE.to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// One chunk: its length, its type, its data, and the CRC of the type and the data.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Read a PNG this module wrote back into its size and pixels — enough of a decoder to prove
    /// the encoder, and no more.
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        assert_eq!(&png[..8], &SIGNATURE);
        let mut at = 8;
        let (mut width, mut height, mut idat) = (0, 0, Vec::new());
        while at < png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind = &png[at + 4..at + 8];
            let data = &png[at + 8..at + 8 + len];
            let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
            let mut check = crc32fast::Hasher::new();
            check.update(kind);
            check.update(data);
            assert_eq!(check.finalize(), crc, "{:?}", std::str::from_utf8(kind));
            match kind {
                b"IHDR" => {
                    width = u32::from_be_bytes(data[..4].try_into().unwrap());
                    height = u32::from_be_bytes(data[4..8].try_into().unwrap());
                    assert_eq!(&data[8..], &[8, 6, 0, 0, 0]);
                }
                b"IDAT" => idat.extend_from_slice(data),
                b"IEND" => break,
                other => panic!("an unexpected chunk {other:?}"),
            }
            at += 12 + len;
        }
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&idat[..])
            .read_to_end(&mut raw)
            .unwrap();
        let stride = width as usize * 4;
        let mut pixels = Vec::new();
        for row in raw.chunks_exact(stride + 1) {
            assert_eq!(row[0], 0, "no filter");
            pixels.extend_from_slice(&row[1..]);
        }
        (width, height, pixels)
    }

    #[test]
    fn pixels_go_in_and_come_back_out() {
        let (w, h) = (3, 2);
        let pixels: Vec<u8> = (0..w * h * 4).map(|i| (i * 7) as u8).collect();
        let png = encode(w, h, &pixels);
        assert_eq!(decode(&png), (w, h, pixels));
    }

    /// Decision 9's premise: the same pixels are the same bytes, every time.
    #[test]
    fn the_same_pixels_are_the_same_bytes() {
        let pixels = vec![0x80; 16 * 16 * 4];
        assert_eq!(encode(16, 16, &pixels), encode(16, 16, &pixels));
    }

    #[test]
    #[should_panic(expected = "one RGBA pixel per place")]
    fn a_short_buffer_is_a_bug() {
        encode(2, 2, &[0; 4]);
    }
}
