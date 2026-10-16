//! Which clips of a track play at a time, with what weight and at what time in their
//! source: eases, blends between overlapping clips, extrapolation and speed.

use crate::asset::{ClipExtrapolation, TimelineClip, TrackAsset};

/// One clip playing on a track at some Timeline time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveClip {
    /// Index into the track's clips.
    pub index: usize,
    /// 0..1 from eases and blends (1 while extrapolating).
    pub weight: f32,
    /// Seconds since the clip's start on the Timeline, after extrapolation (unscaled).
    pub local: f32,
    /// Seconds into the clip's source (`clip_in + local * time_scale`).
    pub source_time: f32,
    /// Outside the clip's own range (Hold, Loop, ... extrapolation).
    pub extrapolated: bool,
}

/// Unity's default ease curve.
fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Blend-in and blend-out lengths each clip gets from overlapping its neighbours (Unity
/// creates a blend wherever two clips overlap on one track).
pub fn blends(clips: &[TimelineClip]) -> Vec<(f32, f32)> {
    let mut out = vec![(0.0f32, 0.0f32); clips.len()];
    for (i, a) in clips.iter().enumerate() {
        for (j, b) in clips.iter().enumerate() {
            // `a` starts first and `b` starts inside it.
            let first = a.start < b.start || (a.start == b.start && i < j);
            if i == j || !first || b.start >= a.end() {
                continue;
            }
            let overlap = (a.end().min(b.end()) - b.start).max(0.0);
            out[j].0 = out[j].0.max(overlap);
            if b.end() >= a.end() {
                out[i].1 = out[i].1.max(overlap);
            }
        }
    }
    out
}

/// Weight of a clip `local` seconds after its start (eases and blends; 0 outside it).
pub fn clip_weight(clip: &TimelineClip, local: f32, blend: (f32, f32)) -> f32 {
    if local < 0.0 || local > clip.duration {
        return 0.0;
    }
    let ein = clip.ease_in_duration.max(blend.0).min(clip.duration);
    let eout = clip.ease_out_duration.max(blend.1).min(clip.duration);
    let mut w = 1.0;
    if ein > 0.0 && local < ein {
        w *= ease(local / ein);
    }
    if eout > 0.0 && local > clip.duration - eout {
        w *= ease((clip.duration - local) / eout);
    }
    w
}

fn extrapolate(mode: ClipExtrapolation, rel: f32, duration: f32) -> Option<f32> {
    let d = duration.max(1e-6);
    Some(match mode {
        ClipExtrapolation::None => return None,
        ClipExtrapolation::Hold => rel.clamp(0.0, duration),
        ClipExtrapolation::Loop => rel.rem_euclid(d),
        ClipExtrapolation::PingPong => {
            let t = rel.rem_euclid(2.0 * d);
            if t > d { 2.0 * d - t } else { t }
        }
        ClipExtrapolation::Continue => rel,
    })
}

fn active(
    clip: &TimelineClip,
    index: usize,
    weight: f32,
    local: f32,
    extrapolated: bool,
) -> ActiveClip {
    ActiveClip {
        index,
        weight,
        local,
        source_time: clip.clip_in + local * clip.time_scale,
        extrapolated,
    }
}

/// The clips of `track` playing at `t` (seconds on the Timeline).
pub fn evaluate_track(track: &TrackAsset, t: f32) -> Vec<ActiveClip> {
    let clips = &track.clips;
    let bl = blends(clips);
    let mut out: Vec<ActiveClip> = Vec::new();
    for (i, c) in clips.iter().enumerate() {
        let local = t - c.start;
        if (0.0..=c.duration).contains(&local) {
            let w = clip_weight(c, local, bl[i]);
            out.push(active(c, i, w, local, false));
        }
    }
    if !out.is_empty() {
        return out;
    }
    // In a gap: the clip before extends forward (post-extrapolation) up to the next clip,
    // otherwise the clip after extends backward (pre-extrapolation).
    let before = clips
        .iter()
        .enumerate()
        .filter(|(_, c)| c.end() < t)
        .max_by(|a, b| a.1.end().total_cmp(&b.1.end()));
    let after = clips
        .iter()
        .enumerate()
        .filter(|(_, c)| c.start > t)
        .min_by(|a, b| a.1.start.total_cmp(&b.1.start));
    if let Some((i, c)) = before
        && let Some(local) = extrapolate(c.post_extrapolation, t - c.start, c.duration)
    {
        return vec![active(c, i, 1.0, local, true)];
    }
    if let Some((i, c)) = after
        && let Some(local) = extrapolate(c.pre_extrapolation, t - c.start, c.duration)
    {
        return vec![active(c, i, 1.0, local, true)];
    }
    out
}

/// Markers crossed moving the playhead from `prev` to `now` (exclusive of `prev`), with
/// wrap-around when the Timeline looped (`now < prev`) over `duration`.
pub fn crossed(times: impl Iterator<Item = f32>, prev: f32, now: f32, duration: f32) -> Vec<usize> {
    times
        .enumerate()
        .filter(|&(_, m)| {
            if now >= prev {
                m > prev && m <= now
            } else {
                (m > prev && m <= duration) || (m >= 0.0 && m <= now)
            }
        })
        .map(|(i, _)| i)
        .collect()
}
