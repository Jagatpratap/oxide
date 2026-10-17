//! Photoshop `.psd` / `.psb` import, like Unity: the flattened composite image (saved with
//! "Maximize Compatibility"), not the layers. Grayscale, RGB and CMYK; 8, 16 and 32 bits;
//! raw or RLE (PackBits). Transparent images have Photoshop's white matte removed (Unity's
//! "Remove Matte (PSD)").
//! Reference: <https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/>

struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or("truncated PSD")?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().expect("2")))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().expect("8")))
    }
    fn skip_section(&mut self, long: bool) -> Result<(), String> {
        let n = if long {
            self.u64()? as usize
        } else {
            self.u32()? as usize
        };
        self.take(n).map(drop)
    }
}

/// PackBits decompression of one row into exactly `out_len` bytes.
fn unpack_bits(src: &[u8], out_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(out_len);
    let mut i = 0;
    while i < src.len() && out.len() < out_len {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let count = n as usize + 1;
            let end = (i + count).min(src.len());
            out.extend_from_slice(&src[i..end]);
            i = end;
        } else if n != -128 {
            let count = (1 - n as isize) as usize;
            if let Some(&v) = src.get(i) {
                out.extend(std::iter::repeat_n(v, count));
            }
            i += 1;
        }
    }
    out.resize(out_len, 0);
    out
}

/// Decodes a PSD/PSB's composite image to `(width, height, RGBA8)`.
pub fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut c = Cursor { b: bytes, pos: 0 };
    if c.take(4)? != b"8BPS" {
        return Err("not a Photoshop file".into());
    }
    let version = c.u16()?;
    let long = version == 2;
    c.take(6)?;
    let channels = c.u16()? as usize;
    let height = c.u32()? as usize;
    let width = c.u32()? as usize;
    let depth = c.u16()? as usize;
    let mode = c.u16()?;
    if !matches!(depth, 8 | 16 | 32) {
        return Err(format!("{depth}-bit PSD images aren't supported"));
    }
    if !matches!(mode, 1 | 3 | 4 | 8) {
        return Err(format!(
            "PSD colour mode {mode} isn't supported (use RGB, Grayscale or CMYK)"
        ));
    }
    // Colour mode data, image resources: 4-byte lengths; layer and mask info: 8 bytes in PSB.
    c.skip_section(false)?;
    c.skip_section(false)?;
    c.skip_section(long)?;
    let compression = c.u16()?;
    let bpc = depth / 8;
    let row_bytes = width * bpc;
    let planes: Vec<Vec<u8>> = match compression {
        0 => (0..channels)
            .map(|_| c.take(row_bytes * height).map(<[u8]>::to_vec))
            .collect::<Result<_, _>>()?,
        1 => {
            let rows = channels * height;
            let counts: Vec<usize> = (0..rows)
                .map(|_| {
                    if long {
                        c.u32().map(|x| x as usize)
                    } else {
                        c.u16().map(|x| x as usize)
                    }
                })
                .collect::<Result<_, _>>()?;
            let mut planes = vec![Vec::with_capacity(row_bytes * height); channels];
            for (r, n) in counts.iter().enumerate() {
                let row = c.take(*n)?;
                planes[r / height].extend(unpack_bits(row, row_bytes));
            }
            planes
        }
        other => {
            return Err(format!(
                "PSD compression {other} (ZIP) isn't supported; save without layer compression"
            ));
        }
    };
    // One channel value as 0..1.
    let sample = |plane: &[u8], i: usize| -> f32 {
        match bpc {
            1 => plane[i] as f32 / 255.0,
            2 => u16::from_be_bytes([plane[i * 2], plane[i * 2 + 1]]) as f32 / 65535.0,
            _ => f32::from_be_bytes(plane[i * 4..i * 4 + 4].try_into().expect("4")).clamp(0.0, 1.0),
        }
    };
    let n = width * height;
    let mut out = vec![0u8; n * 4];
    let (colour, has_alpha) = match mode {
        1 | 8 => (1, channels >= 2),
        3 => (3, channels >= 4),
        _ => (4, channels >= 5),
    };
    if channels < colour {
        return Err("PSD has fewer channels than its colour mode needs".into());
    }
    for i in 0..n {
        let a = if has_alpha {
            sample(&planes[colour], i)
        } else {
            1.0
        };
        let mut rgb = match mode {
            1 | 8 => {
                let g = sample(&planes[0], i);
                [g, g, g]
            }
            3 => [
                sample(&planes[0], i),
                sample(&planes[1], i),
                sample(&planes[2], i),
            ],
            _ => {
                // Photoshop stores CMYK inverted (0 = full ink).
                let (cy, m, y, k) = (
                    1.0 - sample(&planes[0], i),
                    1.0 - sample(&planes[1], i),
                    1.0 - sample(&planes[2], i),
                    1.0 - sample(&planes[3], i),
                );
                [
                    (1.0 - cy) * (1.0 - k),
                    (1.0 - m) * (1.0 - k),
                    (1.0 - y) * (1.0 - k),
                ]
            }
        };
        if has_alpha && a > 0.0 && a < 1.0 {
            // The composite is blended over white: undo it.
            for v in &mut rgb {
                *v = ((*v - (1.0 - a)) / a).clamp(0.0, 1.0);
            }
        }
        let px = &mut out[i * 4..i * 4 + 4];
        px[0] = (rgb[0] * 255.0).round() as u8;
        px[1] = (rgb[1] * 255.0).round() as u8;
        px[2] = (rgb[2] * 255.0).round() as u8;
        px[3] = (a * 255.0).round() as u8;
    }
    Ok((width as u32, height as u32, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(channels: u16, w: u32, h: u32, depth: u16, mode: u16) -> Vec<u8> {
        let mut b = b"8BPS".to_vec();
        b.extend(1u16.to_be_bytes());
        b.extend([0u8; 6]);
        b.extend(channels.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend(w.to_be_bytes());
        b.extend(depth.to_be_bytes());
        b.extend(mode.to_be_bytes());
        b.extend(0u32.to_be_bytes()); // colour mode data
        b.extend(0u32.to_be_bytes()); // image resources
        b.extend(0u32.to_be_bytes()); // layers
        b
    }

    #[test]
    fn raw_rgb_and_rle_rgba() {
        // 2x1 raw RGB: red, blue.
        let mut b = header(3, 2, 1, 8, 3);
        b.extend(0u16.to_be_bytes());
        b.extend([255, 0, 0, 0, 0, 255]); // R plane, G plane, B plane
        let (w, h, px) = decode(&b).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(px, vec![255, 0, 0, 255, 0, 0, 255, 255]);

        // 3x1 RLE RGBA, half-transparent grey over white matte → mid grey after matte removal.
        let mut b = header(4, 3, 1, 8, 3);
        b.extend(1u16.to_be_bytes());
        // Each channel row: a run "repeat next byte 3 times" = [-2 as u8, value].
        let rows: [[u8; 2]; 4] = [[254, 191], [254, 191], [254, 191], [254, 128]];
        for _ in &rows {
            b.extend(2u16.to_be_bytes());
        }
        for r in rows {
            b.extend(r);
        }
        let (_, _, px) = decode(&b).unwrap();
        assert!(px[..3].iter().all(|&c| c.abs_diff(128) <= 1), "{px:?}");
        assert_eq!(px[3], 128);
    }

    #[test]
    fn grayscale_16_bit() {
        let mut b = header(1, 1, 1, 16, 1);
        b.extend(0u16.to_be_bytes());
        b.extend(32768u16.to_be_bytes());
        let (_, _, px) = decode(&b).unwrap();
        assert_eq!(px, vec![128, 128, 128, 255]);
    }
}

#[cfg(test)]
mod compare {
    #[test]
    #[ignore]
    fn against_reference() {
        let (Ok(psd), Ok(png)) = (std::env::var("PSD"), std::env::var("PNG")) else {
            return;
        };
        let (w, h, ours) = super::decode(&std::fs::read(psd).unwrap()).unwrap();
        let reference = image::open(png).unwrap().to_rgba8();
        assert_eq!((w, h), reference.dimensions());
        let mut worst = 0u8;
        for (a, b) in ours.chunks(4).zip(reference.as_raw().chunks(4)) {
            if b[3] > 8 {
                for c in 0..4 {
                    worst = worst.max(a[c].abs_diff(b[c]));
                }
            }
        }
        println!(
            "{w}x{h} worst channel difference on visible pixels: {worst}; sample ours {:?} ref {:?}",
            &ours[(16 * w as usize + 14) * 4..][..4],
            &reference.as_raw()[(16 * w as usize + 14) * 4..][..4]
        );
    }
}
