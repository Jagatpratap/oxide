//! Video decoding: H.264 in MP4 / MOV (OpenH264, built from source), and animated GIFs.
//! A [`FrameSource`] decodes frames in presentation order from a seek point.

use std::io::Cursor;
use std::sync::Arc;

/// What a video holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub frame_count: u64,
    /// Seconds.
    pub length: f64,
    /// The file has an audio track (played through the audio system).
    pub has_audio: bool,
}

/// A decoded picture: presentation time (seconds) and RGBA8 pixels.
#[derive(Clone, Debug)]
pub struct Frame {
    pub time: f64,
    pub rgba: Arc<Vec<u8>>,
}

/// Decodes a video in presentation order.
pub trait FrameSource: Send {
    fn info(&self) -> &VideoInfo;
    /// Restarts decoding at the last key frame at or before `time`; frames before `time`
    /// still come out (the caller skips them).
    fn seek(&mut self, time: f64) -> Result<(), String>;
    /// The next frame, or None at the end.
    fn next_frame(&mut self) -> Result<Option<Frame>, String>;
}

/// Opens a video by its file name's extension (`mp4`, `m4v`, `mov`, `gif`).
pub fn open(bytes: Arc<[u8]>, name: &str) -> Result<Box<dyn FrameSource>, String> {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "gif" => Ok(Box::new(GifSource::new(&bytes)?)),
        "mp4" | "m4v" | "mov" => Ok(Box::new(Mp4Source::new(bytes)?)),
        _ => Err(format!(
            "{name}: unsupported video format (use H.264 MP4/MOV or GIF)"
        )),
    }
}

// ---- GIF ----------------------------------------------------------------------------------

/// Animated GIF (all frames decoded up front).
pub struct GifSource {
    info: VideoInfo,
    frames: Vec<Frame>,
    next: usize,
}

impl GifSource {
    pub fn new(bytes: &[u8]) -> Result<Self, String> {
        use image::AnimationDecoder;
        let dec =
            image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let mut frames = Vec::new();
        let mut t = 0.0;
        let (mut w, mut h) = (0, 0);
        for f in dec.into_frames() {
            let f = f.map_err(|e| e.to_string())?;
            let (num, den) = f.delay().numer_denom_ms();
            let delay = (num as f64 / den.max(1) as f64 / 1000.0).max(0.02);
            let buf = f.into_buffer();
            (w, h) = buf.dimensions();
            frames.push(Frame {
                time: t,
                rgba: Arc::new(buf.into_raw()),
            });
            t += delay;
        }
        if frames.is_empty() {
            return Err("GIF has no frames".into());
        }
        Ok(Self {
            info: VideoInfo {
                width: w,
                height: h,
                frame_rate: frames.len() as f64 / t.max(1e-3),
                frame_count: frames.len() as u64,
                length: t,
                has_audio: false,
            },
            frames,
            next: 0,
        })
    }
}

impl FrameSource for GifSource {
    fn info(&self) -> &VideoInfo {
        &self.info
    }
    fn seek(&mut self, time: f64) -> Result<(), String> {
        self.next = self
            .frames
            .iter()
            .rposition(|f| f.time <= time)
            .unwrap_or(0);
        Ok(())
    }
    fn next_frame(&mut self) -> Result<Option<Frame>, String> {
        let f = self.frames.get(self.next).cloned();
        self.next += 1;
        Ok(f)
    }
}

// ---- H.264 MP4 ----------------------------------------------------------------------------

/// A sample of the video track: decode-order data with its presentation time.
struct Sample {
    pts: f64,
    sync: bool,
}

/// H.264 in an MP4 / MOV container.
pub struct Mp4Source {
    info: VideoInfo,
    reader: mp4::Mp4Reader<Cursor<Arc<[u8]>>>,
    track: u32,
    samples: Vec<Sample>,
    /// SPS and PPS as Annex B (start code + NAL), fed before each key frame.
    headers: Vec<u8>,
    /// AVCC length-field size.
    nal_len: usize,
    decoder: openh264::decoder::Decoder,
    /// Next sample to feed (decode order).
    cursor: usize,
    /// Presentation times of the samples fed since the seek, not yet output.
    pending: Vec<f64>,
    /// Pictures flushed out at the end of the stream, still to return.
    flushed: std::collections::VecDeque<Vec<u8>>,
}

fn avc_nal_length(bytes: &[u8]) -> usize {
    // avcC: lengthSizeMinusOne in the low two bits of its fifth byte (usually 4 bytes).
    let Some(i) = bytes.windows(4).position(|w| w == b"avcC") else {
        return 4;
    };
    bytes.get(i + 4 + 4).map_or(4, |b| (b & 3) as usize + 1)
}

fn to_rgba(yuv: &openh264::decoder::DecodedYUV) -> Vec<u8> {
    use openh264::formats::YUVSource;
    let (w, h) = yuv.dimensions();
    let mut rgba = vec![0u8; w * h * 4];
    yuv.write_rgba8(&mut rgba);
    rgba
}

impl Mp4Source {
    pub fn new(bytes: Arc<[u8]>) -> Result<Self, String> {
        let size = bytes.len() as u64;
        let nal_len = avc_nal_length(&bytes);
        let mut reader =
            mp4::Mp4Reader::read_header(Cursor::new(bytes), size).map_err(|e| e.to_string())?;
        let mut video = None;
        let mut has_audio = false;
        for (id, t) in reader.tracks() {
            match t.track_type() {
                Ok(mp4::TrackType::Video) if matches!(t.media_type(), Ok(mp4::MediaType::H264)) => {
                    video = Some(*id)
                }
                Ok(mp4::TrackType::Audio) => has_audio = true,
                _ => {}
            }
        }
        let track = video.ok_or("no H.264 video track (only H.264 MP4/MOV video is supported)")?;
        let t = &reader.tracks()[&track];
        let mut headers = Vec::new();
        for ps in [t.sequence_parameter_set(), t.picture_parameter_set()] {
            headers.extend_from_slice(&[0, 0, 0, 1]);
            headers.extend_from_slice(ps.map_err(|e| e.to_string())?);
        }
        let timescale = t.timescale().max(1) as f64;
        let (width, height) = (t.width() as u32, t.height() as u32);
        let count = t.sample_count();
        let mut samples = Vec::with_capacity(count as usize);
        for i in 1..=count {
            let s = reader
                .read_sample(track, i)
                .map_err(|e| e.to_string())?
                .ok_or("missing sample")?;
            samples.push(Sample {
                pts: (s.start_time as i64 + s.rendering_offset as i64).max(0) as f64 / timescale,
                sync: s.is_sync,
            });
        }
        let last = samples.iter().map(|s| s.pts).fold(0.0, f64::max);
        let frame_rate = if count > 1 {
            (count - 1) as f64 / last.max(1e-6)
        } else {
            30.0
        };
        Ok(Self {
            info: VideoInfo {
                width,
                height,
                frame_rate,
                frame_count: count as u64,
                length: last + 1.0 / frame_rate.max(1e-3),
                has_audio,
            },
            reader,
            track,
            samples,
            headers,
            nal_len,
            decoder: openh264::decoder::Decoder::new().map_err(|e| e.to_string())?,
            cursor: 0,
            pending: Vec::new(),
            flushed: Default::default(),
        })
    }

    /// AVCC (length-prefixed NALs) → Annex B (start codes), with SPS/PPS before key frames.
    fn annex_b(&self, data: &[u8], key: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 64);
        if key {
            out.extend_from_slice(&self.headers);
        }
        let (mut i, n) = (0, self.nal_len);
        while i + n <= data.len() {
            let len = data[i..i + n]
                .iter()
                .fold(0usize, |a, b| (a << 8) | *b as usize);
            i += n;
            let end = (i + len).min(data.len());
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&data[i..end]);
            i = end;
        }
        out
    }

    /// The presentation time of the next picture out (pictures leave in presentation order).
    fn next_time(&mut self) -> f64 {
        let (i, t) = self
            .pending
            .iter()
            .copied()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((usize::MAX, 0.0));
        if i != usize::MAX {
            self.pending.remove(i);
        }
        t
    }
}

impl FrameSource for Mp4Source {
    fn info(&self) -> &VideoInfo {
        &self.info
    }

    fn seek(&mut self, time: f64) -> Result<(), String> {
        // The last key frame at or before `time`.
        self.cursor = self
            .samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.sync && s.pts <= time + 1e-6)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0);
        self.pending.clear();
        self.flushed.clear();
        self.decoder = openh264::decoder::Decoder::new().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn next_frame(&mut self) -> Result<Option<Frame>, String> {
        if let Some(rgba) = self.flushed.pop_front() {
            let time = self.next_time();
            return Ok(Some(Frame {
                time,
                rgba: Arc::new(rgba),
            }));
        }
        while self.cursor < self.samples.len() {
            let i = self.cursor;
            self.cursor += 1;
            let sample = self
                .reader
                .read_sample(self.track, i as u32 + 1)
                .map_err(|e| e.to_string())?
                .ok_or("missing sample")?;
            let nal = self.annex_b(&sample.bytes, self.samples[i].sync || i == 0);
            self.pending.push(self.samples[i].pts);
            let rgba = self
                .decoder
                .decode(&nal)
                .map_err(|e| format!("H.264: {e}"))?
                .map(|yuv| to_rgba(&yuv));
            if let Some(rgba) = rgba {
                let time = self.next_time();
                return Ok(Some(Frame {
                    time,
                    rgba: Arc::new(rgba),
                }));
            }
        }
        // End of stream: pictures the decoder still holds (B-frame reordering).
        if !self.pending.is_empty() {
            let pics: Vec<Vec<u8>> = self
                .decoder
                .flush_remaining()
                .map_err(|e| format!("H.264: {e}"))?
                .iter()
                .map(to_rgba)
                .collect();
            self.flushed.extend(pics);
            if let Some(rgba) = self.flushed.pop_front() {
                let time = self.next_time();
                return Ok(Some(Frame {
                    time,
                    rgba: Arc::new(rgba),
                }));
            }
            self.pending.clear();
        }
        Ok(None)
    }
}
