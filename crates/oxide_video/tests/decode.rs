//! H.264 MP4 and GIF decoding.

use oxide_video::encode::encode_mp4;
use oxide_video::source::open;
use std::sync::Arc;

/// Frames that get redder over time, with a white square marking the top-left.
fn frames(n: usize, w: u32, h: u32) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| {
            let mut px = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    if x < 8 && y < 8 {
                        px.extend_from_slice(&[255, 255, 255, 255]);
                    } else {
                        px.extend_from_slice(&[(i * 20).min(255) as u8, 40, 200, 255]);
                    }
                }
            }
            px
        })
        .collect()
}

fn red_at(rgba: &[u8], w: u32, x: u32, y: u32) -> u8 {
    rgba[((y * w + x) * 4) as usize]
}

#[test]
fn mp4_h264_decodes_frames_in_order_and_seeks() {
    let (w, h) = (64u32, 48u32);
    let mp4 = encode_mp4(&frames(12, w, h), w, h, 30, 5).unwrap();
    let mut src = open(Arc::from(mp4.as_slice()), "clip.mp4").unwrap();
    let info = src.info().clone();
    assert_eq!((info.width, info.height, info.frame_count), (64, 48, 12));
    assert!((info.frame_rate - 30.0).abs() < 0.5, "{}", info.frame_rate);
    assert!((info.length - 12.0 / 30.0).abs() < 0.01, "{}", info.length);
    assert!(!info.has_audio);
    let mut times = Vec::new();
    while let Some(f) = src.next_frame().unwrap() {
        assert_eq!(f.rgba.len(), (w * h * 4) as usize);
        let i = times.len();
        let r = red_at(&f.rgba, w, 40, 30) as i32;
        assert!(
            (r - (i as i32 * 20).min(255)).abs() < 25,
            "frame {i}: red {r}"
        );
        assert!(red_at(&f.rgba, w, 2, 2) > 200, "the white marker");
        times.push(f.time);
    }
    assert_eq!(times.len(), 12);
    for (i, t) in times.iter().enumerate() {
        assert!((t - i as f64 / 30.0).abs() < 1e-6);
    }
    // Seeking lands on the key frame before the time (key frames every 5).
    src.seek(8.0 / 30.0).unwrap();
    let f = src.next_frame().unwrap().unwrap();
    assert!((f.time - 5.0 / 30.0).abs() < 1e-6, "{}", f.time);
    let r = red_at(&f.rgba, w, 40, 30) as i32;
    assert!((r - 100).abs() < 25, "{r}");
}

#[test]
fn gifs_decode_with_their_delays() {
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame, RgbaImage};
    let mut bytes = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut bytes);
        for c in [[255u8, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]] {
            let img = RgbaImage::from_pixel(8, 8, image::Rgba(c));
            enc.encode_frame(Frame::from_parts(
                img,
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            ))
            .unwrap();
        }
    }
    let mut src = open(Arc::from(bytes.as_slice()), "anim.gif").unwrap();
    assert_eq!(src.info().frame_count, 3);
    assert!((src.info().length - 0.3).abs() < 1e-6);
    let f = src.next_frame().unwrap().unwrap();
    assert_eq!(&f.rgba[..3], &[255, 0, 0]);
    src.seek(0.25).unwrap();
    let f = src.next_frame().unwrap().unwrap();
    assert_eq!((f.time, &f.rgba[..3]), (0.2, &[0u8, 0, 255][..]));
    assert!(open(Arc::from(&b"xx"[..]), "a.webm").is_err());
}

/// Writes a sample clip for manual checks: `OXIDE_WRITE_VIDEO=path cargo test -p
/// oxide_video --test decode -- --ignored`.
#[test]
#[ignore]
fn write_sample_video() {
    let Ok(path) = std::env::var("OXIDE_WRITE_VIDEO") else {
        return;
    };
    let (w, h) = (256u32, 144u32);
    let frames: Vec<Vec<u8>> = (0..90)
        .map(|i| {
            let mut px = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    // A bar sweeping across a gradient.
                    let bar = (x as i32 - (i * 3) % w as i32).abs() < 12;
                    if bar {
                        px.extend_from_slice(&[255, 255, 255, 255]);
                    } else {
                        px.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 160, 255]);
                    }
                }
            }
            px
        })
        .collect();
    std::fs::write(path, encode_mp4(&frames, w, h, 30, 30).unwrap()).unwrap();
}
