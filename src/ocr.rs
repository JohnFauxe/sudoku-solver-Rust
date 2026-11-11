//! `--transcribe`: sudoku image -> text, replicated in Rust from the
//! reference Python tool (`samurai_transcribe.py`). Serializes a puzzle image
//! into the compact text format:
//!
//!     digit (1-9)  -> a printed given
//!     '.'          -> an empty cell
//!     '-'          -> position outside the cross-shaped union of grids
//!
//! both for a classic 9x9 board (81 characters) and for a Samurai Sudoku
//! image (five overlapping 9x9 grids in a 21x21 bounding box, 441 characters):
//!
//! Pipeline (identical to the reference tool):
//!   1. grid detection: strong lines located from ink profiles, working on
//!      light themes (dark ink), dark themes (blue lines, via B > R) and
//!      dark themes with plain gray lines; the bands snap to a 9x9 or a
//!      21x21 lattice, which decides classic vs samurai
//!   2. cell extraction: each in-shape cell is cropped, backgrounded against
//!      its own dominant value, and the glyph's ink taken as a bbox mask
//!   3. clustering: identical glyphs share one cluster (the font is constant
//!      within an image), canonicalized by a LANCZOS resize to 16x24
//!   4. recognition: template matching against the embedded digit templates
//!      with a bijective digits<->clusters assignment (brute force, n <= 9)
//!   5. validation: the result goes through `Grid::parse`, so duplicate or
//!      nonsensical readings surface as warnings
//!
//! The CLI decodes PNG files itself (hand-rolled DEFLATE, no crates); the
//! browser UI hands already-decoded pixels to `POST /api/transcribe`, so it
//! accepts every image format the browser can show.

use std::collections::HashMap;

/// A decoded image, three bytes (RGB) per pixel.
pub struct Raster {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

/// Errors from image decoding or transcription.
#[derive(Debug)]
pub enum OcrError {
    NotPng,
    Png(&'static str),
    PngUnsupported(String),
    Inflate,
    Grid(String),
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OcrError::NotPng => write!(f, "not a PNG image"),
            OcrError::Png(m) => write!(f, "malformed PNG: {m}"),
            OcrError::PngUnsupported(m) => write!(f, "{m}"),
            OcrError::Inflate => write!(f, "corrupt compressed image data"),
            OcrError::Grid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for OcrError {}

// ---------------------------------------------------------------- DEFLATE --

/// LSB-first bit reader over a byte slice.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize, // byte position
    bit: u32,   // bit position within the current byte (0 = LSB)
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0, bit: 0 }
    }
    fn bit(&mut self) -> Result<u32, OcrError> {
        if self.pos >= self.data.len() {
            return Err(OcrError::Inflate);
        }
        let b = (self.data[self.pos] >> self.bit) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        Ok(b as u32)
    }
    /// `n` bits, LSB first.
    fn bits(&mut self, n: u32) -> Result<u32, OcrError> {
        let mut v = 0;
        for i in 0..n {
            v |= self.bit()? << i;
        }
        Ok(v)
    }
    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }
}

/// Canonical Huffman table (puff.c style: per-length counts + ordered symbols).
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Option<Huffman> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        if counts[0] as usize == lengths.len() {
            return None; // no codes at all
        }
        let mut left: i32 = 1;
        for len in 1..=15 {
            left <<= 1;
            left -= counts[len] as i32;
            if left < 0 {
                return None; // over-subscribed
            }
        }
        let mut offs = [0u16; 16];
        for len in 1..15 {
            offs[len + 1] = offs[len] + counts[len];
        }
        let mut symbols = vec![0u16; lengths.len() - counts[0] as usize];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Some(Huffman { counts, symbols })
    }

    fn decode(&self, br: &mut BitReader) -> Result<u16, OcrError> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: usize = 0;
        for len in 1..=15usize {
            code |= br.bit()? as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                return Ok(self.symbols[index + (code - first) as usize]);
            }
            index += count as usize;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(OcrError::Inflate)
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59,
    67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] =
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513,
    769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];
const CLORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn fixed_tables() -> (Huffman, Huffman) {
    let mut lit = [0u8; 288];
    for (i, l) in lit.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    (Huffman::new(&lit).unwrap(), Huffman::new(&[5u8; 30]).unwrap())
}

/// Decode literal/length + distance symbols until the end-of-block marker.
fn copy_huffman(
    lit: &Huffman,
    dist: &Huffman,
    br: &mut BitReader,
    out: &mut Vec<u8>,
) -> Result<(), OcrError> {
    loop {
        match lit.decode(br)? {
            s @ 0..=255 => out.push(s as u8),
            256 => return Ok(()),
            sym => {
                let si = (sym - 257) as usize;
                if si >= 29 {
                    return Err(OcrError::Inflate);
                }
                let len = LEN_BASE[si] as usize + br.bits(LEN_EXTRA[si] as u32)? as usize;
                let ds = dist.decode(br)? as usize;
                if ds >= 30 {
                    return Err(OcrError::Inflate);
                }
                let d = DIST_BASE[ds] as usize + br.bits(DIST_EXTRA[ds] as u32)? as usize;
                if d == 0 || d > out.len() {
                    return Err(OcrError::Inflate);
                }
                let start = out.len() - d;
                for k in 0..len {
                    // byte by byte: the source may overlap the copy
                    let b = out[start + k];
                    out.push(b);
                }
            }
        }
    }
}

fn inflate(br: &mut BitReader, out: &mut Vec<u8>) -> Result<(), OcrError> {
    let mut lit;
    let mut dist;
    loop {
        let bfinal = br.bit()?;
        match br.bits(2)? {
            0 => {
                br.align();
                if br.pos + 4 > br.data.len() {
                    return Err(OcrError::Inflate);
                }
                let len = u16::from_le_bytes([br.data[br.pos], br.data[br.pos + 1]]) as usize;
                let nlen = u16::from_le_bytes([br.data[br.pos + 2], br.data[br.pos + 3]]);
                br.pos += 4;
                if len != (!nlen & 0xffff) as usize || br.pos + len > br.data.len() {
                    return Err(OcrError::Inflate);
                }
                out.extend_from_slice(&br.data[br.pos..br.pos + len]);
                br.pos += len;
            }
            1 => {
                let (l, d) = fixed_tables();
                lit = l;
                dist = d;
                copy_huffman(&lit, &dist, br, out)?;
            }
            2 => {
                let hlit = br.bits(5)? as usize + 257;
                let hdist = br.bits(5)? as usize + 1;
                let hclen = br.bits(4)? as usize + 4;
                let mut cl = [0u8; 19];
                for i in 0..hclen {
                    cl[CLORDER[i]] = br.bits(3)? as u8;
                }
                let Some(cltab) = Huffman::new(&cl) else {
                    return Err(OcrError::Inflate);
                };
                let mut lengths = vec![0u8; hlit + hdist];
                let mut i = 0usize;
                while i < lengths.len() {
                    match cltab.decode(br)? {
                        v @ 0..=15 => {
                            lengths[i] = v as u8;
                            i += 1;
                        }
                        16 => {
                            if i == 0 {
                                return Err(OcrError::Inflate);
                            }
                            let prev = lengths[i - 1];
                            let n = 3 + br.bits(2)? as usize;
                            for _ in 0..n {
                                if i >= lengths.len() {
                                    return Err(OcrError::Inflate);
                                }
                                lengths[i] = prev;
                                i += 1;
                            }
                        }
                        17 => {
                            let n = 3 + br.bits(3)? as usize;
                            i += n; // zeros stay zero
                        }
                        18 => {
                            let n = 11 + br.bits(7)? as usize;
                            i += n;
                        }
                        _ => return Err(OcrError::Inflate),
                    }
                }
                if i > lengths.len() {
                    return Err(OcrError::Inflate);
                }
                let Some(l2) = Huffman::new(&lengths[..hlit]) else {
                    return Err(OcrError::Inflate);
                };
                // An all-zero distance table is legal (all literals); the
                // fixed table substitutes without harm then.
                let d2 = Huffman::new(&lengths[hlit..]).unwrap_or_else(|| fixed_tables().1);
                lit = l2;
                dist = d2;
                copy_huffman(&lit, &dist, br, out)?;
            }
            _ => return Err(OcrError::Inflate),
        }
        if bfinal == 1 {
            break;
        }
    }
    Ok(())
}

/// Inflate a zlib stream (RFC 1950 wrapper + RFC 1951) — used by the PNG
/// decoder and the embedded template blob.
pub fn zlib_decompress(data: &[u8]) -> Result<Vec<u8>, OcrError> {
    if data.len() < 6 {
        return Err(OcrError::Inflate);
    }
    let (cmf, flg) = (data[0], data[1]);
    if cmf & 0x0f != 8 || ((cmf as u16) << 8 | flg as u16) % 31 != 0 || flg & 32 != 0 {
        return Err(OcrError::Inflate);
    }
    let mut br = BitReader::new(&data[2..]);
    let mut out = Vec::new();
    inflate(&mut br, &mut out)?;
    let a = adler32(&out);
    let tail = &data[data.len() - 4..];
    if a != u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]) {
        return Err(OcrError::Inflate);
    }
    Ok(out)
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

// -------------------------------------------------------------------- PNG --

/// Decode a PNG (8-bit, non-interlaced, color types 0/2/3/4/6) into RGB.
/// JPEG and friends are refused with a hint: the browser UI accepts any
/// format because it decodes on the client.
pub fn decode_image(bytes: &[u8]) -> Result<Raster, OcrError> {
    if bytes.len() < 8 {
        return Err(OcrError::NotPng);
    }
    const SIG: [u8; 8] = [137, b'P', b'N', b'G', 13, 10, 26, 10];
    if bytes[..8] != SIG {
        if bytes.starts_with(&[0xFF, 0xD8]) {
            return Err(OcrError::PngUnsupported(
                "JPEG is not supported here (PNG is); the browser UI accepts any format".into(),
            ));
        }
        return Err(OcrError::PngUnsupported(
            "only PNG images are supported here (the browser UI accepts any format)".into(),
        ));
    }
    let mut pos = 8usize;
    let (mut w, mut h, mut ctype, mut depth, mut interlace) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    let mut seen_ihdr = false;
    let mut seen_iend = false;
    while pos + 8 <= bytes.len() {
        let len =
            u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let tag = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        if data_start + len > bytes.len() {
            return Err(OcrError::Png("chunk overruns file"));
        }
        let data = &bytes[data_start..data_start + len];
        match tag {
            b"IHDR" => {
                if len < 13 {
                    return Err(OcrError::Png("short IHDR"));
                }
                w = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                h = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
                depth = data[8];
                ctype = data[9];
                interlace = data[12];
                seen_ihdr = true;
            }
            b"PLTE" => {
                if len % 3 != 0 {
                    return Err(OcrError::Png("bad PLTE"));
                }
                palette = data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => {
                seen_iend = true;
                break;
            }
            _ => {}
        }
        pos = data_start + len + 4; // + CRC (not verified)
    }
    if !seen_ihdr || !seen_iend {
        return Err(OcrError::Png("missing IHDR or IEND"));
    }
    if w == 0 || h == 0 {
        return Err(OcrError::Png("empty image"));
    }
    if depth != 8 {
        return Err(OcrError::PngUnsupported(format!(
            "PNG bit depth {depth} (only 8-bit is supported); re-save the image as 8-bit"
        )));
    }
    if interlace != 0 {
        return Err(OcrError::PngUnsupported(
            "interlaced (Adam7) PNGs are not supported; re-save the image without interlacing"
                .into(),
        ));
    }
    let channels: usize = match ctype {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Err(OcrError::Png("unknown color type")),
    };
    if idat.is_empty() {
        return Err(OcrError::Png("no image data"));
    }
    let raw = zlib_decompress(&idat)?;
    let stride = w * channels;
    if raw.len() < h * (stride + 1) {
        return Err(OcrError::Png("truncated image data"));
    }
    // Unfilter in place (filters 0-4, RFC 2083), row by row.
    let mut img = vec![0u8; h * stride];
    for y in 0..h {
        let f = raw[y * (stride + 1)];
        let off = y * (stride + 1) + 1;
        let line = &raw[off..off + stride];
        let cur = y * stride;
        for x in 0..stride {
            let a = if x >= channels { img[cur + x - channels] } else { 0 };
            let b = if y > 0 { img[cur - stride + x] } else { 0 };
            let c = if y > 0 && x >= channels { img[cur - stride + x - channels] } else { 0 };
            // RFC 2083: the unfilter add is mod-256, so wrap it explicitly
            // (a plain `+` also panics in debug builds).
            img[cur + x] =
                line[x].wrapping_add(match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u32 + b as u32) / 2) as u8,
                4 => {
                    // Paeth: predictor p = a + b - c; pa = |b - c|,
                    // pb = |a - c|, pc = |a + b - 2c| (RFC 2083)
                    let (pa, pb, pc) = (
                        (b as i16 - c as i16).abs(),
                        (a as i16 - c as i16).abs(),
                        (a as i32 + b as i32 - 2 * c as i32).abs() as i16,
                    );
                    let pred = if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c };
                    pred as u8
                }
                _ => return Err(OcrError::Png("bad filter type")),
            });
        }
    }
    // Convert to RGB.
    let mut rgb = vec![0u8; w * h * 3];
    for i in 0..w * h {
        let o = i * 3;
        match ctype {
            0 => {
                let v = img[i];
                rgb[o] = v;
                rgb[o + 1] = v;
                rgb[o + 2] = v;
            }
            2 => rgb[o..o + 3].copy_from_slice(&img[i * 3..i * 3 + 3]),
            3 => {
                let idx = img[i] as usize;
                let Some(p) = palette.get(idx) else {
                    return Err(OcrError::Png("palette index out of range"));
                };
                rgb[o] = p[0];
                rgb[o + 1] = p[1];
                rgb[o + 2] = p[2];
            }
            4 => {
                let v = img[i * 2];
                rgb[o] = v;
                rgb[o + 1] = v;
                rgb[o + 2] = v;
            }
            6 => {
                let (mut rr, mut gg, mut bb) = (img[i * 4], img[i * 4 + 1], img[i * 4 + 2]);
                let a = img[i * 4 + 3];
                if a != 255 {
                    // composite over white, like the page background
                    rr = over_white(rr, a);
                    gg = over_white(gg, a);
                    bb = over_white(bb, a);
                }
                rgb[o] = rr;
                rgb[o + 1] = gg;
                rgb[o + 2] = bb;
            }
            _ => unreachable!(),
        }
    }
    Ok(Raster { w, h, rgb })
}

fn over_white(v: u8, a: u8) -> u8 {
    ((v as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8
}

/// Read an image file for the CLI (PNG only).
pub fn decode_file(path: &str) -> Result<Raster, OcrError> {
    let bytes =
        std::fs::read(path).map_err(|_| OcrError::PngUnsupported(format!("cannot read {path}")))?;
    decode_image(&bytes)
}

// ------------------------------------------------- transcription pipeline --

pub const GRID: usize = 21;
const CELL_W: usize = 16;
const CELL_H: usize = 24;
const CLUSTER_TOL: i64 = 30 * (CELL_W * CELL_H) as i64;
const INK_MIN: u32 = 12;
const DIGIT_TOL: i32 = 70;

/// cell (r, c) belongs to the samurai union iff both cross conditions hold
fn in_shape(r: usize, c: usize) -> bool {
    (r < 9 || r > 11 || (6..=14).contains(&c)) && (c < 9 || c > 11 || (6..=14).contains(&r))
}

/// Smallest value holding the majority (numpy `bincount().argmax()`).
fn mode(vals: &[u8]) -> u8 {
    let mut hist = [0u32; 256];
    for &v in vals {
        hist[v as usize] += 1;
    }
    let (mut best, mut idx) = (0u32, 0u8);
    for (v, &c) in hist.iter().enumerate() {
        if c > best {
            best = c;
            idx = v as u8;
        }
    }
    idx
}

struct Geom {
    k: usize, // 9 = classic 9x9 board, 21 = samurai 21x21 cross
    cols: Vec<f32>,
    rows: Vec<f32>,
}

/// Line band centers from an ink profile (consecutive runs average to
/// `(first + last) / 2`, matching the reference tool).
fn line_bands(profile: &[u32], thr: f64) -> Vec<usize> {
    let mut bands = Vec::new();
    let mut run: Option<(usize, usize)> = None;
    for (i, &v) in profile.iter().enumerate() {
        if v as f64 > thr {
            match run {
                Some((s, _)) => run = Some((s, i)),
                None => run = Some((i, i)),
            }
        } else if let Some((s, prev)) = run.take() {
            bands.push((s + prev) / 2);
        }
    }
    if let Some((s, prev)) = run.take() {
        bands.push((s + prev) / 2);
    }
    bands
}

/// Map line bands onto lattice indices `0..=k` with uniform spacing, bridging
/// renders that only draw the box borders by interpolation; returns None when
/// the bands don't snap to the lattice cleanly.
fn map_axis(bands: &[usize], k: usize) -> Option<Vec<f32>> {
    if bands.len() < 3 || bands[0] == bands[bands.len() - 1] {
        return None;
    }
    let first = bands[0] as f64;
    let last = *bands.last().unwrap() as f64;
    let cell = (last - first) / k as f64;
    if cell < 5.0 {
        return None;
    }
    // merge bands closer than half a cell (double-detected lines)
    let mut merged: Vec<f64> = Vec::new();
    for &b in bands {
        if let Some(p) = merged.last_mut() {
            if b as f64 - *p < (cell * 0.45).max(2.0) {
                *p = (*p + b as f64) / 2.0;
                continue;
            }
        }
        merged.push(b as f64);
    }
    if merged.len() > k + 1 {
        return None;
    }
    if merged.len() < 8 {
        // too few anchors to trust the lattice: fall back to uniform pitch
        return None;
    }
    let mut at = vec![None::<f64>; k + 1];
    for &b in &merged {
        let e = (b - first) / cell;
        let i = e.round() as i64;
        if i < 0 || i > k as i64 || (e - i as f64).abs() > 0.2 || at[i as usize].is_some() {
            return None;
        }
        at[i as usize] = Some(b);
    }
    let mut out = vec![0f32; k + 1];
    let mut prev: Option<(i64, f64)> = None;
    for i in 0..=k {
        let Some(y) = at[i] else { continue };
        let i = i as i64;
        match prev {
            None => {
                for j in 0..i {
                    out[j as usize] = (y - (i - j) as f64 * cell) as f32;
                }
            }
            Some((pi, py)) => {
                for j in (pi + 1)..i {
                    out[j as usize] = (py + (y - py) * (j - pi) as f64 / (i - pi) as f64) as f32;
                }
            }
        }
        out[i as usize] = y as f32;
        prev = Some((i, y));
    }
    if let Some((pi, py)) = prev {
        for j in ((pi + 1) as usize)..=k {
            out[j] = (py + (j - pi as usize) as f64 * cell) as f32;
        }
    }
    Some(out)
}

fn find_grid(r: &Raster, gray: &[u8]) -> Result<Geom, OcrError> {
    let (w, h) = (r.w, r.h);
    let bg = mode(gray);
    // Ink = pixels far from the background in gray, plus blue grid lines on
    // dark themes, whose gray value is often too close to the background to
    // stand out (the "dark theme" of the reference tool: blue lattice,
    // near-white text). The retries with looser thresholds are for the thin
    // inner cell lines that render just barely above the background while
    // the bold box borders already pass the strict pass (the in-page OCR
    // retries its Otsu threshold the same way).
    let mut fallback: Option<Geom> = None;
    for (tol, btol) in [(60i32, 40i32), (40, 25), (20, 10)] {
        let far: Vec<bool> =
            gray.iter().map(|&g| (g as i32 - bg as i32).abs() > tol).collect();
        let dark: Vec<bool> = if bg > 128 {
            far
        } else {
            r.rgb
                .chunks_exact(3)
                .zip(far)
                .map(|(c, f)| f || c[2] as i32 > c[0] as i32 + btol)
                .collect()
        };
        let vprofile: Vec<u32> =
            (0..w).map(|x| (0..h).filter(|&y| dark[y * w + x]).count() as u32).collect();
        let hprofile: Vec<u32> =
            (0..h).map(|y| (0..w).filter(|&x| dark[y * w + x]).count() as u32).collect();
        let vl = line_bands(&vprofile, h as f64 * 0.4);
        let hl = line_bands(&hprofile, w as f64 * 0.4);
        if vl.len() >= 8 && hl.len() >= 8 && vl[vl.len() - 1] != vl[0] && hl[hl.len() - 1] != hl[0]
        {
            // Classic 9x9 first, then the samurai 21x21 cross: the correct
            // lattice snaps the line bands cleanly, the other one misses
            // every interior line by a fraction of a pitch and can't map.
            for k in [9usize, 21] {
                if let (Some(cols), Some(rows)) = (map_axis(&vl, k), map_axis(&hl, k)) {
                    return Ok(Geom { k, cols, rows });
                }
            }
            // Uniform-pitch fallback from the outermost lines, shape picked
            // by the number of lines: a full 9x9 board has 10 per axis, a
            // samurai cross 22. Remembered in case no pass ever maps.
            if fallback.is_none() {
                let k = if vl.len() <= 10 && hl.len() <= 10 { 9 } else { 21 };
                let px = (vl[vl.len() - 1] - vl[0] + 1) / k;
                let py = (hl[hl.len() - 1] - hl[0] + 1) / k;
                fallback = Some(Geom {
                    k,
                    cols: (0..=k).map(|i| (vl[0] + px * i) as f32).collect(),
                    rows: (0..=k).map(|i| (hl[0] + py * i) as f32).collect(),
                });
            }
        }
    }
    fallback.ok_or_else(|| {
        OcrError::Grid(
            "could not locate a sudoku grid (classic 9x9 or samurai 21x21) in this image".into(),
        )
    })
}

/// LANCZOS (windowed sinc, support 3) resample, mirroring Pillow's
/// `Image.LANCZOS`, of an 8-bit grayscale image.
fn lanczos_resize(src: &[u8], w: usize, h: usize, ow: usize, oh: usize) -> Vec<u8> {
    fn kernel(x: f64) -> f64 {
        if x == 0.0 {
            1.0
        } else {
            let ax = x.abs();
            if ax >= 3.0 {
                0.0
            } else {
                let px = std::f64::consts::PI * x;
                (px.sin() * (px / 3.0).sin())
                    / (std::f64::consts::PI * std::f64::consts::PI * x * x)
            }
        }
    }
    fn pass(src: &[u8], in_len: usize, out_len: usize) -> Vec<f64> {
        let scale = out_len as f64 / in_len as f64;
        let filter_scale = if scale < 1.0 { 1.0 / scale } else { 1.0 };
        let support = 3.0 * filter_scale;
        let mut out = vec![0f64; out_len];
        for i in 0..out_len {
            let center = (i as f64 + 0.5) / scale - 0.5;
            let lo = ((center - support + 1.0).ceil() as i64).max(0) as usize;
            let hi = (((center + support).floor() as i64).min(in_len as i64 - 1)).max(0) as usize;
            let mut sum = 0f64;
            let mut weights = Vec::with_capacity(hi.saturating_sub(lo) + 1);
            for x in lo..=hi {
                let wk = kernel((x as f64 - center) / filter_scale);
                weights.push(wk);
                sum += wk;
            }
            if sum != 0.0 {
                for wk in weights.iter_mut() {
                    *wk /= sum;
                }
            }
            out[i] = (lo..=hi)
                .map(|x| weights[x - lo] * src[x] as f64)
                .sum::<f64>()
                .clamp(0.0, 255.0);
        }
        out
    }
    let mut work = vec![0f64; ow * h];
    for y in 0..h {
        let out = pass(&src[y * w..(y + 1) * w], w, ow);
        work[y * ow..(y + 1) * ow].copy_from_slice(&out);
    }
    let mut out = vec![0u8; ow * oh];
    for x in 0..ow {
        let col: Vec<u8> = (0..h).map(|y| work[y * ow + x] as u8).collect();
        let outcol = pass(&col, h, oh);
        for y in 0..oh {
            out[y * ow + x] = outcol[y].round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[derive(Clone)]
struct Glyph {
    r: usize,
    c: usize,
    norm: Vec<u8>, // canonical 16x24 mask
}

/// A transcription result: the text (81 chars for a classic grid, 441 for
/// samurai) plus metadata for the UI.
pub struct Transcription {
    /// 9 = classic 9x9, 21 = samurai 21x21
    pub k: usize,
    pub text: String,
    pub warnings: Vec<String>,
    /// display position of every recognized digit (for preview overlays)
    pub cells: Vec<(usize, usize)>,
    /// `cells` whose digit could not be named (rendered '?')
    pub unknown: Vec<(usize, usize)>,
    /// refined line positions in pixels: `rows[r]`, `cols[c]` for 0..=k
    pub rows: Vec<f32>,
    pub cols: Vec<f32>,
}

/// Display cells of a given lattice size that carry clues: for the samurai
/// cross the union of the five 9x9 grids, for a classic board everything.
fn in_lattice(k: usize, r: usize, c: usize) -> bool {
    k == 9 || in_shape(r, c)
}

/// Transcribe a sudoku image (classic 9x9 or samurai 21x21).
pub fn transcribe(r: &Raster) -> Result<Transcription, OcrError> {
    let w = r.w;
    let h = r.h;
    let gray: Vec<u8> = r
        .rgb
        .chunks_exact(3)
        .map(|c| ((c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000) as u8)
        .collect();
    let geom = find_grid(r, &gray)?;

    let mut glyphs: Vec<Glyph> = Vec::new();
    for cr in 0..geom.k {
        for cc in 0..geom.k {
            if !in_lattice(geom.k, cr, cc) {
                continue;
            }
            let top = (geom.rows[cr] as i64 + 3).min(h as i64);
            let bot = (geom.rows[cr + 1] as i64 - 2).min(h as i64);
            let left = (geom.cols[cc] as i64 + 3).min(w as i64);
            let right = (geom.cols[cc + 1] as i64 - 2).min(w as i64);
            if bot <= top || right <= left {
                continue;
            }
            let (cw, ch) = ((right - left) as usize, (bot - top) as usize);
            let mut region = Vec::with_capacity(cw * ch);
            for y in top..bot {
                let base = y as usize * w;
                region.extend_from_slice(&gray[base + left as usize..base + right as usize]);
            }
            let cbg = mode(&region);
            let mask: Vec<bool> =
                region.iter().map(|&g| (g as i32 - cbg as i32).abs() > DIGIT_TOL).collect();
            let ink = mask.iter().filter(|&&m| m).count() as u32;
                    if ink < INK_MIN {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (cw, ch, 0usize, 0usize);
            for y in 0..ch {
                for x in 0..cw {
                    if mask[y * cw + x] {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x);
                        y1 = y1.max(y);
                    }
                }
            }
            let (gw, gh) = (x1 - x0 + 1, y1 - y0 + 1);
            let mut gmask = vec![0u8; gw * gh];
            for y in 0..gh {
                for x in 0..gw {
                    if mask[(y + y0) * cw + x + x0] {
                        gmask[y * gw + x] = 255;
                    }
                }
            }
            let norm = lanczos_resize(&gmask, gw, gh, CELL_W, CELL_H);
            glyphs.push(Glyph { r: cr, c: cc, norm });
        }
    }

    // Cluster identical glyphs (first match within tolerance wins, as in the
    // reference tool; the font is constant within an image).
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        let mut placed = false;
        for cl in clusters.iter_mut() {
            if dist_i64(&g.norm, &glyphs[cl[0]].norm) < CLUSTER_TOL {
                cl.push(i);
                placed = true;
                break;
            }
        }
        if !placed {
            clusters.push(vec![i]);
        }
    }
    let labels = label_clusters(&clusters, &glyphs);
    let mut label_of: HashMap<(usize, usize), char> = HashMap::new();
    for (ci, cl) in clusters.iter().enumerate() {
        for &idx in cl {
            label_of.insert((glyphs[idx].r, glyphs[idx].c), labels[ci]);
        }
    }

    let mut text = String::with_capacity(geom.k * geom.k);
    let mut cells = Vec::new();
    let mut unknown = Vec::new();
    for r in 0..geom.k {
        for c in 0..geom.k {
            if !in_lattice(geom.k, r, c) {
                text.push('-');
            } else if let Some(&ch) = label_of.get(&(r, c)) {
                text.push(ch);
                if ch == '?' {
                    unknown.push((r, c));
                } else {
                    cells.push((r, c));
                }
            } else {
                text.push('.');
            }
        }
    }

    // Validation via the app's own parser: duplicates and stray characters
    // surface as warnings (the reference tool's consistency check).
    let warnings: Vec<String> = match crate::grid::Grid::parse(&text) {
        Ok(_) => Vec::new(),
        Err(e) => vec![e.to_string()],
    };

    Ok(Transcription { k: geom.k, text, warnings, cells, unknown, rows: geom.rows, cols: geom.cols })
}

fn dist_i64(a: &[u8], b: &[u8]) -> i64 {
    a.iter().zip(b.iter()).map(|(&x, &y)| (x as i64 - y as i64).abs()).sum()
}

/// Assign a digit to each cluster: nearest-template cost, solved for the
/// bijection digits<->clusters by brute force in lexicographic permutation
/// order (first minimum wins, matching itertools.permutations). Clusters
/// beyond nine (degenerate clustering) stay '?'.
fn label_clusters(clusters: &[Vec<usize>], glyphs: &[Glyph]) -> Vec<char> {
    let n = clusters.len();
    if n > 9 {
        return vec!['?'; n];
    }
    if n == 0 {
        return Vec::new();
    }
    let templates = crate::ocr_data::templates();
    let mut cost = vec![[i64::MAX; 9]; n];
    for (i, cl) in clusters.iter().enumerate() {
        let rep = &glyphs[cl[0]].norm;
        for (j, tbl) in templates.iter().enumerate() {
            if let Some(c) = tbl.iter().map(|t| dist_i64(rep, t)).min() {
                cost[i][j] = c;
            }
        }
    }
    let mut used = [false; 9];
    let mut perm = vec![0usize; n];
    let mut best_score = i64::MAX;
    let mut best_perm = perm.clone();
    fn rec(
        cost: &[[i64; 9]],
        used: &mut [bool; 9],
        perm: &mut [usize],
        depth: usize,
        score: i64,
        best_score: &mut i64,
        best_perm: &mut [usize],
    ) {
        if depth == perm.len() {
            if score < *best_score {
                *best_score = score;
                best_perm.copy_from_slice(perm);
            }
            return;
        }
        for d in 0..9 {
            if used[d] {
                continue;
            }
            used[d] = true;
            perm[depth] = d;
            rec(
                cost,
                used,
                perm,
                depth + 1,
                score.saturating_add(cost[depth][d]),
                best_score,
                best_perm,
            );
            used[d] = false;
        }
    }
    rec(&cost, &mut used, &mut perm, 0, 0, &mut best_score, &mut best_perm);
    let mut labels = vec!['?'; n];
    for (i, &d) in best_perm.iter().enumerate() {
        if cost[i][d] != i64::MAX {
            labels[i] = char::from_digit(d as u32 + 1, 10).unwrap();
        }
    }
    labels
}

/// Read an image file and transcribe it; used by `--transcribe`.
pub fn transcribe_file(path: &str) -> Result<Transcription, OcrError> {
    transcribe(&decode_file(path)?)
}

/// Readable row-by-row rendering of a transcription (the `--pretty` view).
/// The shape (and so the segmenting of each row) follows the text length:
/// 81 characters = classic 9x9, 441 = samurai.
pub fn pretty_grid(text: &str, warnings: &[String]) -> String {
    let k = if text.len() == 81 { 9 } else { GRID };
    let segs: Vec<(usize, usize)> =
        if k == 9 { vec![(0, 9)] } else { vec![(0, 9), (9, 12), (12, 21)] };
    let width = k.to_string().len().max(2);
    let mut out = Vec::new();
    for r in 0..k {
        let row = &text[r * k..(r + 1) * k];
        let disp = segs
            .iter()
            .map(|&(s, e)| {
                row[s..e]
                    .as_bytes()
                    .chunks(3)
                    .map(|c| String::from_utf8_lossy(c).to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("  ");
        out.push(format!("{:width$} {disp}", r));
    }
    for wt in warnings {
        out.push(format!("WARNING: {wt}"));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn inflate_stored_fixed_dynamic() {
        // all vectors produced with python3 zlib.compress, verified once
        assert_eq!(zlib_decompress(&hex("789c4b4c4c040002490124")).unwrap(), b"aaa");
        // LZ77 with an overlapping copy: 1000 * 'a'
        assert_eq!(
            zlib_decompress(&hex("78da4b4c1c05a360140c770000f9d87af8")).unwrap(),
            vec![b'a'; 1000]
        );
        // fixed-huffman block
        assert_eq!(
            zlib_decompress(&hex("7801f348cdc9c9d751c800518a001fd00476")).unwrap(),
            b"Hello, hello!"
        );
        // dynamic huffman: 00..09 repeated 20 times
        let plain: Vec<u8> = (0..10u8).cycle().take(200).collect();
        assert_eq!(
            zlib_decompress(&hex("78da6360646266616563e7e06418d22c005bb70385")).unwrap(),
            plain
        );
        // wrong adler32 trailer must be rejected
        assert!(zlib_decompress(&hex("789c4b4c4c040002490100")).is_err());
        assert!(zlib_decompress(&hex("7800")).is_err());
    }

    #[test]
    fn png_color_types_and_filters() {
        // 3x2 grayscale, rows ff 80 00 / 00 40 ff
        let r = decode_image(&hex(
            "89504e470d0a1a0a0000000d4948445200000003000000020800000000b81f39c6000000104944415478da63f8dfc0c0c0e0f01f000b8002bfb47f3b4b0000000049454e44ae426082",
        ))
        .unwrap();
        assert_eq!((r.w, r.h), (3, 2));
        assert_eq!(r.rgb, vec![255, 255, 255, 128, 128, 128, 0, 0, 0, 0, 0, 0, 64, 64, 64, 255, 255, 255]);
        // 2x1 RGB: red, green
        let r = decode_image(&hex(
            "89504e470d0a1a0a0000000d49484452000000020000000108020000007b40e8dd0000000f4944415478da63f8cfc0c0f09f010007ff01ffb80435e00000000049454e44ae426082",
        ))
        .unwrap();
        assert_eq!(r.rgb, vec![255, 0, 0, 0, 255, 0]);
        // 2x1 palette: PLTE [red, blue], indices 0 1
        let r = decode_image(&hex(
            "89504e470d0a1a0a0000000d4948445200000002000000010803000000c3fc8fb800000006504c5445ff00000000ff6ca1fd8e0000000b4944415478da6360600400000400022cde48ad0000000049454e44ae426082",
        ))
        .unwrap();
        assert_eq!(r.rgb, vec![255, 0, 0, 0, 0, 255]);
        // 2x1 grayscale+alpha: opaque white, opaque black
        let r = decode_image(&hex(
            "89504e470d0a1a0a0000000d49484452000000020000000108040000005e2bb7010000000d4944415478da63f8dfc0f01f000680027fcf1209f10000000049454e44ae426082",
        ))
        .unwrap();
        assert_eq!(r.rgb, vec![255, 255, 255, 0, 0, 0]);
        // 2x1 RGBA: opaque red + green at alpha 64 over white -> gray 191
        let r = decode_image(&hex(
            "89504e470d0a1a0a0000000d4948445200000002000000010806000000f4227f8a0000000f4944415478da63f8cfc0f01f081d001039033e14075ddd0000000049454e44ae426082",
        ))
        .unwrap();
        assert_eq!(r.rgb, vec![255, 0, 0, 191, 255, 191]);
        // 2x1 RGB with the Sub filter: raw bytes 01 00 00 00 | 02 00 00
        let r = decode_image(&hex(
            "89504e470d0a1a0a0000000d49484452000000020000000108020000007b40e8dd0000000f4944415478da63646060606260000000140004c1730bfa0000000049454e44ae426082",
        ))
        .unwrap();
        assert_eq!(r.rgb, vec![0, 0, 0, 2, 0, 0]);
    }

    #[test]
    fn refuses_non_png() {
        assert!(matches!(decode_image(b"gif89aXYZ"), Err(OcrError::PngUnsupported(_))));
        assert!(matches!(
            decode_image(&[0xff, 0xd8, 0xff, 0xe0, 0, 0, 0, 0]),
            Err(OcrError::PngUnsupported(_))
        ));
        assert!(matches!(decode_image(b"xx"), Err(OcrError::NotPng)));
    }

    #[test]
    fn shape_matches_sample_mask() {
        let per_row: Vec<usize> = (0..GRID)
            .map(|r| (0..GRID).filter(|&c| !in_shape(r, c)).count())
            .collect();
        assert_eq!(per_row, vec![3, 3, 3, 3, 3, 3, 0, 0, 0, 12, 12, 12, 0, 0, 0, 3, 3, 3, 3, 3, 3]);
        assert_eq!((0..GRID * GRID).filter(|&i| in_shape(i / GRID, i % GRID)).count(), 369);
    }

    #[test]
    fn band_mapping_and_fallback() {
        // full lattice: 22 lines at exact pitch
        let bands: Vec<usize> = (0..=21).map(|i| 5 + i * 27).collect();
        let cols = map_axis(&bands, 21).expect("maps");
        assert_eq!(cols[0], 5.0);
        assert_eq!(cols[13], 5.0 + 13.0 * 27.0);
        assert_eq!(cols[21], 5.0 + 21.0 * 27.0);
        // box-only lattice: rows 0, 3, ..., 21 only
        let box_bands: Vec<usize> = (0..=21).filter(|i| i % 3 == 0).map(|i| 10 + i * 31).collect();
        let cols = map_axis(&box_bands, 21).expect("maps box-only");
        assert_eq!(cols[1], 10.0 + 31.0); // interpolated
        assert_eq!(cols[15], 10.0 + 15.0 * 31.0);
        assert_eq!(cols[21], 10.0 + 21.0 * 31.0);
        // un-snap-able spacing: caller falls back to uniform pitch
        assert!(map_axis(&[4, 6, 1000], 21).is_none());
    }

    #[test]
    fn classic_vs_samurai_lattice() {
        // A 9x9 board's ten lines sit exactly on a 9-lattice and not on a
        // 21-lattice (every interior line misses by over 0.2 pitch); a
        // samurai cross's 22 lines are the exact opposite.
        let classic: Vec<usize> = (0..=9).map(|i| 10 + i * 50).collect();
        assert!(map_axis(&classic, 9).is_some());
        assert!(map_axis(&classic, 21).is_none());
        let samurai: Vec<usize> = (0..=21).map(|i| 10 + i * 27).collect();
        assert!(map_axis(&samurai, 9).is_none());
        assert!(map_axis(&samurai, 21).is_some());
    }

    #[test]
    fn pretty_classic_nine_rows() {
        let text = format!("{}1{}", ".".repeat(26), ".".repeat(54));
        let p = pretty_grid(&text, &[]);
        let lines: Vec<&str> = p.lines().collect();
        assert_eq!(lines.len(), 9);
        assert!(lines[2].contains("1"));
        assert!(!p.contains('-'));
    }

    #[test]
    fn mode_breaks_ties_low() {
        assert_eq!(mode(&[9, 9, 7, 7]), 7);
        assert_eq!(mode(&[5]), 5);
    }

    #[test]
    fn assignment_is_bijective_and_lex_stable() {
        // glyphs taken straight from the embedded templates: two clusters
        // shaped like the digits 1 and 7 must come out labeled '1' and '7'
        let templates = crate::ocr_data::templates();
        let g1 = Glyph { r: 0, c: 0, norm: templates[0][0].clone() };
        let g7 = Glyph { r: 1, c: 2, norm: templates[6][0].clone() };
        assert_eq!(label_clusters(&[vec![0], vec![1]], &[g1.clone(), g7.clone()]), vec!['1', '7']);
        assert_eq!(label_clusters(&[vec![1], vec![0]], &[g1.clone(), g7.clone()]), vec!['7', '1']);
        // degenerate clustering: more clusters than digits -> all '?'
        let glyphs10: Vec<Glyph> =
            (0..10).map(|i| Glyph { r: i % 2, c: 0, norm: templates[0][i % 90].clone() }).collect();
        let many: Vec<Vec<usize>> = (0..10).map(|k| vec![k]).collect();
        assert_eq!(label_clusters(&many, &glyphs10), vec!['?'; 10]);
        assert_eq!(label_clusters(&[], &[]), Vec::<char>::new());
    }

    #[test]
    fn pretty_view_uses_spaces() {
        let text = format!("1{}", ".".repeat(GRID * GRID - 1));
        let p = pretty_grid(&text, &["boom".to_string()]);
        assert!(p.lines().next().unwrap().starts_with(" 0 1.. .."));
        assert!(p.contains("WARNING: boom"));
    }
}
