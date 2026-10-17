//! Unity's Audio Random Container: an asset of clips an Audio Source plays (as its resource)
//! in sequence, shuffled or at random, with randomised volume, pitch and timing. Good for
//! footsteps, gunshots, impacts: no two in a row sound the same.
//!
//! Manual trigger: each `Play` plays one clip. Automatic trigger: `Play` starts a stream of
//! clips, one every `trigger_time` (Pulse) or `trigger_time` after the previous one ends
//! (Offset), until the loop (Infinite / a number of clips / a number of cycles) runs out.
//! Reference: <https://docs.unity3d.com/Manual/AudioRandomContainer.html>

use oxide_assets::{Asset, AudioClip, Handle};
use oxide_core::Reflect;
use std::collections::VecDeque;

/// One clip of a container.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct AudioContainerElement {
    pub clip: Option<Handle<AudioClip>>,
    /// Volume offset (dB).
    pub volume: f32,
    pub enabled: bool,
}

impl Default for AudioContainerElement {
    fn default() -> Self {
        Self {
            clip: None,
            volume: 0.0,
            enabled: true,
        }
    }
}

/// How the next clip is chosen.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioContainerPlaybackMode {
    /// In list order.
    Sequential,
    /// Every clip once in a random order, then a new order.
    Shuffle,
    /// Any clip, except the last `avoid_repeating_last`.
    #[default]
    Random,
}

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioContainerTriggerMode {
    /// Each `Play` plays one clip.
    #[default]
    Manual,
    /// `Play` starts playing clips on a timer until the loop ends or `Stop`.
    Automatic,
}

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioContainerAutomaticTriggerMode {
    /// A clip every `trigger_time` seconds (clips may overlap).
    #[default]
    Pulse,
    /// The next clip `trigger_time` seconds after the previous one ends.
    Offset,
}

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioContainerLoopMode {
    #[default]
    Infinite,
    /// Stop after `loop_count` clips.
    Clips,
    /// Stop after going through the list `loop_count` times.
    Cycles,
}

/// Unity's Audio Random Container asset (`.randomcontainer`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct AudioRandomContainer {
    pub elements: Vec<AudioContainerElement>,
    /// Volume (dB) and its random offset range (dB).
    pub volume: f32,
    pub volume_randomization: bool,
    pub volume_random_min: f32,
    pub volume_random_max: f32,
    /// Pitch (cents) and its random offset range (cents).
    pub pitch: f32,
    pub pitch_randomization: bool,
    pub pitch_random_min: f32,
    pub pitch_random_max: f32,
    pub playback_mode: AudioContainerPlaybackMode,
    /// Random mode: clips that can't come up again for this many picks.
    pub avoid_repeating_last: u32,
    pub trigger: AudioContainerTriggerMode,
    pub automatic_trigger_mode: AudioContainerAutomaticTriggerMode,
    /// Automatic trigger: seconds between clips (Pulse) or after each clip (Offset).
    pub trigger_time: f32,
    pub time_randomization: bool,
    pub time_random_min: f32,
    pub time_random_max: f32,
    pub loop_mode: AudioContainerLoopMode,
    pub loop_count: u32,
}

impl Default for AudioRandomContainer {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            volume: 0.0,
            volume_randomization: false,
            volume_random_min: -3.0,
            volume_random_max: 0.0,
            pitch: 0.0,
            pitch_randomization: false,
            pitch_random_min: -100.0,
            pitch_random_max: 100.0,
            playback_mode: AudioContainerPlaybackMode::Random,
            avoid_repeating_last: 1,
            trigger: AudioContainerTriggerMode::Manual,
            automatic_trigger_mode: AudioContainerAutomaticTriggerMode::Pulse,
            trigger_time: 1.0,
            time_randomization: false,
            time_random_min: -0.1,
            time_random_max: 0.1,
            loop_mode: AudioContainerLoopMode::Infinite,
            loop_count: 3,
        }
    }
}

impl Asset for AudioRandomContainer {
    const TYPE_NAME: &'static str = "AudioRandomContainer";
}

/// A clip to play: the element, its volume (dB) and playback rate.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerPick {
    pub element: usize,
    pub volume_db: f32,
    pub rate: f32,
}

/// One Audio Source's playback of a container: what was played, the shuffle order and the
/// automatic trigger's timer.
#[derive(Clone, Debug)]
pub struct ContainerPlayer {
    rng: u64,
    next_sequential: usize,
    shuffle: Vec<usize>,
    history: VecDeque<usize>,
    /// Automatic trigger running.
    pub active: bool,
    /// Seconds until the next automatic clip.
    timer: f32,
    played: u32,
}

impl ContainerPlayer {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: seed | 1,
            next_sequential: 0,
            shuffle: Vec::new(),
            history: VecDeque::new(),
            active: false,
            timer: 0.0,
            played: 0,
        }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, a: f32, b: f32) -> f32 {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        lo + (hi - lo) * self.unit()
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// The next element by the playback mode, with randomised volume and pitch. `None` when
    /// no element has a clip.
    pub fn next(&mut self, c: &AudioRandomContainer) -> Option<ContainerPick> {
        let usable: Vec<usize> = (0..c.elements.len())
            .filter(|&i| c.elements[i].enabled && c.elements[i].clip.is_some())
            .collect();
        if usable.is_empty() {
            return None;
        }
        let element = match c.playback_mode {
            AudioContainerPlaybackMode::Sequential => {
                let i = usable
                    .iter()
                    .copied()
                    .find(|&i| i >= self.next_sequential)
                    .unwrap_or(usable[0]);
                self.next_sequential = i + 1;
                i
            }
            AudioContainerPlaybackMode::Shuffle => {
                self.shuffle.retain(|i| usable.contains(i));
                if self.shuffle.is_empty() {
                    let mut order = usable.clone();
                    for k in (1..order.len()).rev() {
                        let j = self.below(k + 1);
                        order.swap(k, j);
                    }
                    // A new round doesn't start with the clip the last one ended on.
                    let n = order.len();
                    if n > 1 && self.history.back() == order.first() {
                        order.swap(0, n - 1);
                    }
                    order.reverse();
                    self.shuffle = order;
                }
                self.shuffle.pop().unwrap_or(usable[0])
            }
            AudioContainerPlaybackMode::Random => {
                let avoid = (c.avoid_repeating_last as usize).min(usable.len() - 1);
                let recent: Vec<usize> = self.history.iter().rev().take(avoid).copied().collect();
                let allowed: Vec<usize> = usable
                    .iter()
                    .copied()
                    .filter(|i| !recent.contains(i))
                    .collect();
                allowed[self.below(allowed.len())]
            }
        };
        self.history.push_back(element);
        while self.history.len() > c.elements.len().max(1) {
            self.history.pop_front();
        }
        let mut volume_db = c.volume + c.elements[element].volume;
        if c.volume_randomization {
            volume_db += self.range(c.volume_random_min, c.volume_random_max);
        }
        let mut cents = c.pitch;
        if c.pitch_randomization {
            cents += self.range(c.pitch_random_min, c.pitch_random_max);
        }
        Some(ContainerPick {
            element,
            volume_db,
            rate: 2f32.powf(cents / 1200.0),
        })
    }

    /// Starts playing: automatic containers start their timer (the first clip plays now).
    pub fn start(&mut self, c: &AudioRandomContainer) {
        self.active = c.trigger == AudioContainerTriggerMode::Automatic;
        self.timer = 0.0;
        self.played = 0;
    }

    pub fn stop(&mut self) {
        self.active = false;
    }

    /// Whether the loop allows another clip.
    fn loop_allows(&self, c: &AudioRandomContainer) -> bool {
        let usable = c
            .elements
            .iter()
            .filter(|e| e.enabled && e.clip.is_some())
            .count() as u32;
        match c.loop_mode {
            AudioContainerLoopMode::Infinite => true,
            AudioContainerLoopMode::Clips => self.played < c.loop_count,
            AudioContainerLoopMode::Cycles => self.played < c.loop_count * usable,
        }
    }

    /// Advances the automatic trigger by `dt`; returns the clip to play now, if it's time.
    /// `duration` gives an element's clip length (seconds) for the Offset mode.
    pub fn update(
        &mut self,
        c: &AudioRandomContainer,
        dt: f32,
        duration: impl Fn(usize) -> f32,
    ) -> Option<ContainerPick> {
        if !self.active {
            return None;
        }
        if self.timer > 0.0 {
            self.timer -= dt;
            // (A small tolerance keeps float drift from slipping a trigger by a frame.)
            if self.timer > 1e-4 {
                return None;
            }
        }
        if !self.loop_allows(c) {
            self.active = false;
            return None;
        }
        let pick = self.next(c)?;
        self.played += 1;
        let mut wait = c.trigger_time;
        if c.time_randomization {
            wait += self.range(c.time_random_min, c.time_random_max);
        }
        if c.automatic_trigger_mode == AudioContainerAutomaticTriggerMode::Offset {
            wait += duration(pick.element) / pick.rate.max(0.01);
        }
        // Keeps the pulse steady when a frame overshoots the trigger.
        self.timer = self.timer.min(0.0) + wait.max(0.01);
        Some(pick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(n: usize) -> AudioRandomContainer {
        AudioRandomContainer {
            elements: (0..n)
                .map(|i| AudioContainerElement {
                    clip: Some(Handle::from_path(format!("Assets/step{i}.wav"))),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn sequential_shuffle_and_random_picks() {
        let mut c = container(4);
        c.playback_mode = AudioContainerPlaybackMode::Sequential;
        c.elements[2].enabled = false;
        let mut p = ContainerPlayer::new(7);
        let seq: Vec<usize> = (0..5).map(|_| p.next(&c).unwrap().element).collect();
        assert_eq!(seq, [0, 1, 3, 0, 1], "skips disabled clips and wraps");

        c.elements[2].enabled = true;
        c.playback_mode = AudioContainerPlaybackMode::Shuffle;
        let mut p = ContainerPlayer::new(42);
        let mut last = None;
        for _ in 0..10 {
            let mut round: Vec<usize> = (0..4).map(|_| p.next(&c).unwrap().element).collect();
            assert_ne!(
                Some(round[0]),
                last,
                "a round doesn't repeat the previous clip"
            );
            last = round.last().copied();
            round.sort();
            assert_eq!(round, [0, 1, 2, 3], "every clip once per round");
        }

        c.playback_mode = AudioContainerPlaybackMode::Random;
        c.avoid_repeating_last = 2;
        let mut p = ContainerPlayer::new(3);
        let picks: Vec<usize> = (0..200).map(|_| p.next(&c).unwrap().element).collect();
        for w in picks.windows(3) {
            assert!(w[2] != w[1] && w[2] != w[0], "{w:?}");
        }
        for i in 0..4 {
            assert!(picks.contains(&i));
        }
        assert!(ContainerPlayer::new(1).next(&container(0)).is_none());
    }

    #[test]
    fn randomises_volume_and_pitch() {
        let mut c = container(1);
        c.volume = -6.0;
        c.elements[0].volume = 2.0;
        c.pitch = 1200.0;
        let mut p = ContainerPlayer::new(9);
        let pick = p.next(&c).unwrap();
        assert_eq!((pick.volume_db, pick.rate), (-4.0, 2.0), "an octave up");
        c.pitch = 0.0;
        c.volume_randomization = true;
        c.pitch_randomization = true;
        for _ in 0..100 {
            let pick = p.next(&c).unwrap();
            assert!((-7.0..=-4.0).contains(&pick.volume_db));
            let cents = 1200.0 * pick.rate.log2();
            assert!((-100.01..=100.01).contains(&cents), "{cents}");
        }
    }

    #[test]
    fn automatic_trigger_pulse_offset_and_loops() {
        let mut c = container(2);
        c.trigger = AudioContainerTriggerMode::Automatic;
        c.trigger_time = 0.5;
        c.loop_mode = AudioContainerLoopMode::Clips;
        c.loop_count = 3;
        let mut p = ContainerPlayer::new(5);
        let mut times = Vec::new();
        p.start(&c);
        for f in 0..100 {
            if p.update(&c, 0.1, |_| 0.0).is_some() {
                times.push(f);
            }
        }
        assert_eq!(times, [0, 5, 10], "a clip every 0.5 s, three clips");
        assert!(!p.active, "the loop ended");

        c.loop_mode = AudioContainerLoopMode::Cycles;
        c.loop_count = 2;
        c.automatic_trigger_mode = AudioContainerAutomaticTriggerMode::Offset;
        c.trigger_time = 0.2;
        p.start(&c);
        times.clear();
        for f in 0..100 {
            if p.update(&c, 0.1, |_| 1.0).is_some() {
                times.push(f);
            }
        }
        assert_eq!(
            times,
            [0, 12, 24, 36],
            "1 s clip + 0.2 s, two cycles of two clips"
        );

        c.loop_mode = AudioContainerLoopMode::Infinite;
        p.start(&c);
        assert!(p.update(&c, 0.1, |_| 1.0).is_some());
        p.stop();
        assert!(p.update(&c, 5.0, |_| 1.0).is_none());
        c.trigger = AudioContainerTriggerMode::Manual;
        p.start(&c);
        assert!(!p.active, "manual containers play one clip per Play");
    }
}
