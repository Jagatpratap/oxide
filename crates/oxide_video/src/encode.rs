//! Writes H.264 MP4 files from RGBA frames (OpenH264 encoder + MP4 muxer): for tests,
//! tools and recorded clips.

use std::io::Cursor;

/// Encodes `frames` (RGBA8, `width` × `height`, even sizes) at `fps` into an MP4, with a
/// key frame every `key_interval` frames.
pub fn encode_mp4(
    frames: &[Vec<u8>],
    width: u32,
    height: u32,
    fps: u32,
    key_interval: u32,
) -> Result<Vec<u8>, String> {
    use openh264::encoder::{Encoder, EncoderConfig, FrameRate, IntraFramePeriod};
    use openh264::formats::{RgbaSliceU8, YUVBuffer};
    let config = EncoderConfig::new()
        .max_frame_rate(FrameRate::from_hz(fps as f32))
        .intra_frame_period(IntraFramePeriod::from_num_frames(key_interval.max(1)));
    let api = openh264::OpenH264API::from_source();
    let mut enc = Encoder::with_api_config(api, config).map_err(|e| e.to_string())?;
    let (mut sps, mut pps) = (Vec::new(), Vec::new());
    let mut samples: Vec<(Vec<u8>, bool)> = Vec::new();
    for (i, rgba) in frames.iter().enumerate() {
        if (i as u32).is_multiple_of(key_interval.max(1)) {
            enc.force_intra_frame();
        }
        let yuv =
            YUVBuffer::from_rgb_source(RgbaSliceU8::new(rgba, (width as usize, height as usize)));
        let bs = enc.encode(&yuv).map_err(|e| e.to_string())?;
        let mut avcc = Vec::new();
        let mut key = false;
        for l in 0..bs.num_layers() {
            let layer = bs.layer(l).ok_or("layer")?;
            for n in 0..layer.nal_count() {
                let nal = layer.nal_unit(n).ok_or("nal")?;
                // Strip the start code.
                let body = if nal.starts_with(&[0, 0, 0, 1]) {
                    &nal[4..]
                } else if nal.starts_with(&[0, 0, 1]) {
                    &nal[3..]
                } else {
                    nal
                };
                let Some(&h) = body.first() else { continue };
                match h & 0x1f {
                    7 => sps = body.to_vec(),
                    8 => pps = body.to_vec(),
                    t => {
                        key |= t == 5;
                        avcc.extend_from_slice(&(body.len() as u32).to_be_bytes());
                        avcc.extend_from_slice(body);
                    }
                }
            }
        }
        samples.push((avcc, key));
    }
    let config = mp4::Mp4Config {
        major_brand: "isom".parse().map_err(|_| "brand")?,
        minor_version: 512,
        compatible_brands: vec![
            "isom".parse().map_err(|_| "brand")?,
            "avc1".parse().map_err(|_| "brand")?,
        ],
        timescale: 1000,
    };
    let mut w =
        mp4::Mp4Writer::write_start(Cursor::new(Vec::new()), &config).map_err(|e| e.to_string())?;
    w.add_track(&mp4::TrackConfig {
        track_type: mp4::TrackType::Video,
        timescale: fps,
        language: "und".into(),
        media_conf: mp4::MediaConfig::AvcConfig(mp4::AvcConfig {
            width: width as u16,
            height: height as u16,
            seq_param_set: sps,
            pic_param_set: pps,
        }),
    })
    .map_err(|e| e.to_string())?;
    for (i, (data, key)) in samples.into_iter().enumerate() {
        w.write_sample(
            1,
            &mp4::Mp4Sample {
                start_time: i as u64,
                duration: 1,
                rendering_offset: 0,
                is_sync: key,
                bytes: bytes::Bytes::from(data),
            },
        )
        .map_err(|e| e.to_string())?;
    }
    w.write_end().map_err(|e| e.to_string())?;
    Ok(w.into_writer().into_inner())
}
