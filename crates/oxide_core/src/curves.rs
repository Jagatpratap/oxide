//! Unity's `AnimationCurve`, `Gradient`, `ParticleSystem.MinMaxCurve` and `MinMaxGradient`:
//! editable graphs for torque by RPM, tyre grip by slip, size over lifetime, and so on.
//! References: <https://docs.unity3d.com/ScriptReference/AnimationCurve.html>,
//! <https://docs.unity3d.com/ScriptReference/Gradient.html>,
//! <https://docs.unity3d.com/ScriptReference/ParticleSystem.MinMaxCurve.html>

use crate::Reflect;
use oxide_math::Color;

/// How one side of a key's tangent is computed (Unity's `AnimationUtility.TangentMode`).
#[derive(
    Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub enum TangentMode {
    /// Smooth, never overshooting the neighbouring keys (Unity's default, "Clamped Auto").
    #[default]
    ClampedAuto,
    /// Smooth Catmull-Rom style slope; may overshoot.
    Auto,
    /// Set by hand.
    Free,
    /// Straight line to the neighbouring key.
    Linear,
    /// Holds the value until the next key (step).
    Constant,
}

/// What a curve does before its first key and after its last (Unity's `WrapMode`).
#[derive(
    Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub enum CurveWrapMode {
    #[default]
    Clamp,
    Loop,
    PingPong,
}

/// One point of an [`AnimationCurve`] (Unity's `Keyframe`). Tangents are slopes
/// (value per unit of time).
#[derive(Default, Reflect, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Keyframe {
    pub time: f32,
    pub value: f32,
    pub in_tangent: f32,
    pub out_tangent: f32,
    pub in_mode: TangentMode,
    pub out_mode: TangentMode,
    /// Unity's "Broken": the two sides can have different slopes.
    pub broken: bool,
}

impl Keyframe {
    pub fn new(time: f32, value: f32) -> Self {
        Self {
            time,
            value,
            ..Default::default()
        }
    }

    pub fn with_tangents(time: f32, value: f32, in_tangent: f32, out_tangent: f32) -> Self {
        Self {
            time,
            value,
            in_tangent,
            out_tangent,
            in_mode: TangentMode::Free,
            out_mode: TangentMode::Free,
            broken: in_tangent != out_tangent,
        }
    }
}

/// A curve of keys joined by cubic Hermite segments (Unity's `AnimationCurve`).
#[derive(Reflect, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[reflect(default)]
pub struct AnimationCurve {
    pub keys: Vec<Keyframe>,
    pub pre_wrap_mode: CurveWrapMode,
    pub post_wrap_mode: CurveWrapMode,
}

impl Default for AnimationCurve {
    /// A straight line from (0, 0) to (1, 1), like a new curve field in Unity.
    fn default() -> Self {
        Self::linear(0.0, 0.0, 1.0, 1.0)
    }
}

impl AnimationCurve {
    /// A curve through `keys`, sorted by time, with automatic tangents filled in.
    pub fn new(keys: impl IntoIterator<Item = Keyframe>) -> Self {
        let mut c = Self {
            keys: keys.into_iter().collect(),
            pre_wrap_mode: CurveWrapMode::Clamp,
            post_wrap_mode: CurveWrapMode::Clamp,
        };
        c.sort_and_update();
        c
    }

    /// Unity `AnimationCurve.Constant`.
    pub fn constant(t0: f32, t1: f32, value: f32) -> Self {
        Self::linear(t0, value, t1, value)
    }

    /// Unity `AnimationCurve.Linear`: a straight line between two keys.
    pub fn linear(t0: f32, v0: f32, t1: f32, v1: f32) -> Self {
        let mut a = Keyframe::new(t0, v0);
        let mut b = Keyframe::new(t1, v1);
        for k in [&mut a, &mut b] {
            k.in_mode = TangentMode::Linear;
            k.out_mode = TangentMode::Linear;
        }
        Self::new([a, b])
    }

    /// Unity `AnimationCurve.EaseInOut`: flat at both ends.
    pub fn ease_in_out(t0: f32, v0: f32, t1: f32, v1: f32) -> Self {
        Self::new([
            Keyframe::with_tangents(t0, v0, 0.0, 0.0),
            Keyframe::with_tangents(t1, v1, 0.0, 0.0),
        ])
    }

    /// A curve through `(time, value)` points with smooth automatic tangents.
    pub fn smooth(points: &[(f32, f32)]) -> Self {
        Self::new(points.iter().map(|&(t, v)| Keyframe::new(t, v)))
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Time range covered by the keys (`(0, 1)` for an empty curve).
    pub fn time_range(&self) -> (f32, f32) {
        match (self.keys.first(), self.keys.last()) {
            (Some(a), Some(b)) => (a.time, b.time),
            _ => (0.0, 1.0),
        }
    }

    /// Lowest and highest value of the drawn curve, sampled (for framing a graph).
    pub fn value_range(&self) -> (f32, f32) {
        if self.keys.is_empty() {
            return (0.0, 1.0);
        }
        let (t0, t1) = self.time_range();
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for i in 0..=64 {
            let v = self.evaluate(t0 + (t1 - t0) * i as f32 / 64.0);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        for k in &self.keys {
            lo = lo.min(k.value);
            hi = hi.max(k.value);
        }
        (lo, hi)
    }

    /// Adds a key (Unity `AddKey`); returns its index after sorting.
    pub fn add_key(&mut self, key: Keyframe) -> usize {
        let i = self.keys.partition_point(|k| k.time < key.time);
        self.keys.insert(i, key);
        self.update_tangents();
        i
    }

    /// Adds a key on the curve at `time` (its current value), like double-clicking a
    /// curve in Unity.
    pub fn add_key_at(&mut self, time: f32) -> usize {
        let value = self.evaluate(time);
        self.add_key(Keyframe::new(time, value))
    }

    pub fn remove_key(&mut self, index: usize) {
        if index < self.keys.len() {
            self.keys.remove(index);
            self.update_tangents();
        }
    }

    /// Moves a key (Unity `MoveKey`); returns its new index.
    pub fn move_key(&mut self, index: usize, time: f32, value: f32) -> usize {
        if index >= self.keys.len() {
            return index;
        }
        let mut k = self.keys.remove(index);
        k.time = time;
        k.value = value;
        let i = self.keys.partition_point(|o| o.time < time);
        self.keys.insert(i, k);
        self.update_tangents();
        i
    }

    /// Sorts keys by time and recomputes automatic and linear tangents.
    pub fn sort_and_update(&mut self) {
        self.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.update_tangents();
    }

    /// Recomputes every tangent that isn't `Free` from the neighbouring keys.
    pub fn update_tangents(&mut self) {
        let n = self.keys.len();
        for i in 0..n {
            let prev = (i > 0).then(|| self.keys[i - 1]);
            let next = (i + 1 < n).then(|| self.keys[i + 1]);
            let k = self.keys[i];
            let slope = |a: Keyframe, b: Keyframe| {
                let dt = b.time - a.time;
                if dt.abs() < 1e-6 {
                    0.0
                } else {
                    (b.value - a.value) / dt
                }
            };
            let auto = |clamped: bool| match (prev, next) {
                (Some(p), Some(nx)) => {
                    let dt = nx.time - p.time;
                    let s = if dt.abs() < 1e-6 {
                        0.0
                    } else {
                        (nx.value - p.value) / dt
                    };
                    // Clamped: flat at local peaks and valleys, so the curve never
                    // overshoots its keys.
                    if clamped
                        && ((k.value >= p.value && k.value >= nx.value)
                            || (k.value <= p.value && k.value <= nx.value))
                    {
                        0.0
                    } else {
                        s
                    }
                }
                _ => 0.0,
            };
            let side = |mode: TangentMode, current: f32, neighbour: Option<Keyframe>, out: bool| {
                match mode {
                    TangentMode::ClampedAuto => auto(true),
                    TangentMode::Auto => auto(false),
                    TangentMode::Free => current,
                    TangentMode::Linear => {
                        neighbour.map_or(0.0, |o| if out { slope(k, o) } else { slope(o, k) })
                    }
                    TangentMode::Constant => 0.0,
                }
            };
            let mut inn = side(k.in_mode, k.in_tangent, prev, false);
            let mut out = side(k.out_mode, k.out_tangent, next, true);
            // An unbroken key whose sides are both automatic or one free side keeps one
            // slope through the key.
            if !k.broken {
                match (k.in_mode, k.out_mode) {
                    (TangentMode::Free, TangentMode::Free) => out = inn,
                    (TangentMode::Free, _) => out = inn,
                    (_, TangentMode::Free) => inn = out,
                    _ => {}
                }
            }
            self.keys[i].in_tangent = inn;
            self.keys[i].out_tangent = out;
        }
    }

    /// Unity `AnimationCurve.Evaluate`.
    pub fn evaluate(&self, time: f32) -> f32 {
        let n = self.keys.len();
        if n == 0 {
            return 0.0;
        }
        if n == 1 {
            return self.keys[0].value;
        }
        let (t0, t1) = self.time_range();
        let len = t1 - t0;
        let t = if time < t0 {
            wrap(self.pre_wrap_mode, time, t0, len)
        } else if time > t1 {
            wrap(self.post_wrap_mode, time, t0, len)
        } else {
            time
        };
        if t <= t0 {
            return self.keys[0].value;
        }
        if t >= t1 {
            return self.keys[n - 1].value;
        }
        let i = self.keys.partition_point(|k| k.time <= t).clamp(1, n - 1);
        let a = self.keys[i - 1];
        let b = self.keys[i];
        hermite(a, b, t)
    }
}

fn wrap(mode: CurveWrapMode, t: f32, t0: f32, len: f32) -> f32 {
    if len <= 0.0 {
        return t0;
    }
    match mode {
        CurveWrapMode::Clamp => t.clamp(t0, t0 + len),
        CurveWrapMode::Loop => t0 + (t - t0).rem_euclid(len),
        CurveWrapMode::PingPong => {
            let x = (t - t0).rem_euclid(2.0 * len);
            t0 + if x > len { 2.0 * len - x } else { x }
        }
    }
}

/// Cubic Hermite between two keys; a Constant tangent on either side means a step.
fn hermite(a: Keyframe, b: Keyframe, t: f32) -> f32 {
    let dt = b.time - a.time;
    if dt <= 0.0 {
        return b.value;
    }
    if a.out_mode == TangentMode::Constant || b.in_mode == TangentMode::Constant {
        return a.value;
    }
    let s = (t - a.time) / dt;
    let s2 = s * s;
    let s3 = s2 * s;
    let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
    let h10 = s3 - 2.0 * s2 + s;
    let h01 = -2.0 * s3 + 3.0 * s2;
    let h11 = s3 - s2;
    h00 * a.value + h10 * dt * a.out_tangent + h01 * b.value + h11 * dt * b.in_tangent
}

/// Gradient colour key (Unity's `GradientColorKey`); alpha is ignored.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GradientColorKey {
    pub color: Color,
    pub time: f32,
}

/// Gradient alpha key (Unity's `GradientAlphaKey`).
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GradientAlphaKey {
    pub alpha: f32,
    pub time: f32,
}

/// Unity's `GradientMode`.
#[derive(
    Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub enum GradientMode {
    #[default]
    Blend,
    /// Each key's colour holds until the next key.
    Fixed,
}

/// Unity's `Gradient`: colour and alpha keys over 0..1, up to 8 of each.
#[derive(Reflect, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[reflect(default)]
pub struct Gradient {
    pub color_keys: Vec<GradientColorKey>,
    pub alpha_keys: Vec<GradientAlphaKey>,
    pub mode: GradientMode,
}

impl Default for Gradient {
    /// Opaque white, like a new gradient field in Unity.
    fn default() -> Self {
        Self::solid(Color::WHITE)
    }
}

impl Gradient {
    pub const MAX_KEYS: usize = 8;

    pub fn solid(c: Color) -> Self {
        Self::two(c, c)
    }

    /// A blend from `a` at 0 to `b` at 1, alpha included.
    pub fn two(a: Color, b: Color) -> Self {
        Self {
            color_keys: vec![
                GradientColorKey {
                    color: a,
                    time: 0.0,
                },
                GradientColorKey {
                    color: b,
                    time: 1.0,
                },
            ],
            alpha_keys: vec![
                GradientAlphaKey {
                    alpha: a.a,
                    time: 0.0,
                },
                GradientAlphaKey {
                    alpha: b.a,
                    time: 1.0,
                },
            ],
            mode: GradientMode::Blend,
        }
    }

    /// Unity `Gradient.Evaluate`.
    pub fn evaluate(&self, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let rgb = sample(
            &self
                .color_keys
                .iter()
                .map(|k| (k.time, [k.color.r, k.color.g, k.color.b]))
                .collect::<Vec<_>>(),
            t,
            self.mode,
        )
        .unwrap_or([1.0; 3]);
        let a = sample(
            &self
                .alpha_keys
                .iter()
                .map(|k| (k.time, [k.alpha]))
                .collect::<Vec<_>>(),
            t,
            self.mode,
        )
        .map_or(1.0, |a| a[0]);
        Color::new(rgb[0], rgb[1], rgb[2], a)
    }

    /// Sorts keys by time (the editor calls this after dragging).
    pub fn sort(&mut self) {
        self.color_keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.alpha_keys.sort_by(|a, b| a.time.total_cmp(&b.time));
    }
}

fn sample<const N: usize>(
    keys: &[(f32, [f32; N])],
    t: f32,
    mode: GradientMode,
) -> Option<[f32; N]> {
    let mut sorted = keys.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let first = *sorted.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    for w in sorted.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            if mode == GradientMode::Fixed {
                // Unity's Fixed mode shows each key's colour up to (and including) its time.
                return Some(if t <= a.0 { a.1 } else { b.1 });
            }
            let s = if b.0 > a.0 {
                (t - a.0) / (b.0 - a.0)
            } else {
                1.0
            };
            let mut out = a.1;
            for (o, bv) in out.iter_mut().zip(b.1) {
                *o += (bv - *o) * s;
            }
            return Some(out);
        }
    }
    sorted.last().map(|k| k.1)
}

/// Unity's `ParticleSystemCurveMode`.
#[derive(
    Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub enum MinMaxCurveMode {
    #[default]
    Constant,
    Curve,
    RandomBetweenTwoCurves,
    RandomBetweenTwoConstants,
}

/// Unity's `ParticleSystem.MinMaxCurve`: a constant, a curve, or a random value between
/// two constants or two curves.
#[derive(Reflect, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[reflect(default)]
pub struct MinMaxCurve {
    pub mode: MinMaxCurveMode,
    pub constant: f32,
    pub constant_min: f32,
    pub constant_max: f32,
    /// Scales the curve(s); the curves themselves usually span 0..1.
    pub curve_multiplier: f32,
    pub curve: AnimationCurve,
    pub curve_min: AnimationCurve,
    pub curve_max: AnimationCurve,
}

impl Default for MinMaxCurve {
    fn default() -> Self {
        Self::constant(1.0)
    }
}

impl MinMaxCurve {
    pub fn constant(v: f32) -> Self {
        Self {
            mode: MinMaxCurveMode::Constant,
            constant: v,
            constant_min: v,
            constant_max: v,
            curve_multiplier: 1.0,
            curve: AnimationCurve::constant(0.0, 1.0, 1.0),
            curve_min: AnimationCurve::constant(0.0, 1.0, 0.0),
            curve_max: AnimationCurve::constant(0.0, 1.0, 1.0),
        }
    }

    pub fn random(min: f32, max: f32) -> Self {
        Self {
            mode: MinMaxCurveMode::RandomBetweenTwoConstants,
            constant_min: min,
            constant_max: max,
            ..Self::constant(max)
        }
    }

    pub fn curve(multiplier: f32, curve: AnimationCurve) -> Self {
        Self {
            mode: MinMaxCurveMode::Curve,
            curve_multiplier: multiplier,
            curve,
            ..Self::constant(multiplier)
        }
    }

    /// Value at normalized `t` (e.g. particle age / lifetime); `random` in 0..1 picks
    /// between min and max (keep it fixed per particle).
    pub fn evaluate(&self, t: f32, random: f32) -> f32 {
        let lerp = |a: f32, b: f32| a + (b - a) * random;
        match self.mode {
            MinMaxCurveMode::Constant => self.constant,
            MinMaxCurveMode::RandomBetweenTwoConstants => {
                lerp(self.constant_min, self.constant_max)
            }
            MinMaxCurveMode::Curve => self.curve.evaluate(t) * self.curve_multiplier,
            MinMaxCurveMode::RandomBetweenTwoCurves => {
                lerp(self.curve_min.evaluate(t), self.curve_max.evaluate(t)) * self.curve_multiplier
            }
        }
    }

    /// Largest value it can produce (for bounds).
    pub fn max_value(&self) -> f32 {
        match self.mode {
            MinMaxCurveMode::Constant => self.constant,
            MinMaxCurveMode::RandomBetweenTwoConstants => self.constant_min.max(self.constant_max),
            MinMaxCurveMode::Curve => self.curve.value_range().1 * self.curve_multiplier,
            MinMaxCurveMode::RandomBetweenTwoCurves => {
                self.curve_min
                    .value_range()
                    .1
                    .max(self.curve_max.value_range().1)
                    * self.curve_multiplier
            }
        }
    }
}

/// Unity's `ParticleSystemGradientMode`.
#[derive(
    Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
pub enum MinMaxGradientMode {
    #[default]
    Color,
    Gradient,
    RandomBetweenTwoColors,
    RandomBetweenTwoGradients,
    /// A random colour from the gradient.
    RandomColor,
}

/// Unity's `ParticleSystem.MinMaxGradient`.
#[derive(Reflect, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[reflect(default)]
pub struct MinMaxGradient {
    pub mode: MinMaxGradientMode,
    pub color: Color,
    pub color_min: Color,
    pub color_max: Color,
    pub gradient: Gradient,
    pub gradient_min: Gradient,
    pub gradient_max: Gradient,
}

impl Default for MinMaxGradient {
    fn default() -> Self {
        Self::color(Color::WHITE)
    }
}

impl MinMaxGradient {
    pub fn color(c: Color) -> Self {
        Self {
            mode: MinMaxGradientMode::Color,
            color: c,
            color_min: c,
            color_max: c,
            gradient: Gradient::solid(c),
            gradient_min: Gradient::solid(c),
            gradient_max: Gradient::solid(c),
        }
    }

    pub fn gradient(g: Gradient) -> Self {
        Self {
            mode: MinMaxGradientMode::Gradient,
            gradient: g,
            ..Self::color(Color::WHITE)
        }
    }

    /// Colour at normalized `t`; `random` in 0..1 picks between the two ends.
    pub fn evaluate(&self, t: f32, random: f32) -> Color {
        let lerp = |a: Color, b: Color| {
            Color::new(
                a.r + (b.r - a.r) * random,
                a.g + (b.g - a.g) * random,
                a.b + (b.b - a.b) * random,
                a.a + (b.a - a.a) * random,
            )
        };
        match self.mode {
            MinMaxGradientMode::Color => self.color,
            MinMaxGradientMode::Gradient => self.gradient.evaluate(t),
            MinMaxGradientMode::RandomBetweenTwoColors => lerp(self.color_min, self.color_max),
            MinMaxGradientMode::RandomBetweenTwoGradients => {
                lerp(self.gradient_min.evaluate(t), self.gradient_max.evaluate(t))
            }
            MinMaxGradientMode::RandomColor => self.gradient.evaluate(random),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::{FieldValue, Value};

    #[test]
    fn linear_and_constant() {
        let c = AnimationCurve::linear(0.0, 0.0, 2.0, 4.0);
        assert!((c.evaluate(1.0) - 2.0).abs() < 1e-5);
        assert_eq!(c.evaluate(-1.0), 0.0, "clamped before the first key");
        assert_eq!(c.evaluate(5.0), 4.0, "clamped after the last key");
        assert_eq!(AnimationCurve::constant(0.0, 1.0, 3.0).evaluate(0.4), 3.0);
        let mut step = AnimationCurve::linear(0.0, 1.0, 1.0, 5.0);
        step.keys[0].out_mode = TangentMode::Constant;
        step.update_tangents();
        assert_eq!(step.evaluate(0.99), 1.0, "constant tangent holds the value");
    }

    #[test]
    fn ease_in_out_is_flat_at_the_ends() {
        let c = AnimationCurve::ease_in_out(0.0, 0.0, 1.0, 1.0);
        assert!((c.evaluate(0.5) - 0.5).abs() < 1e-5);
        assert!(c.evaluate(0.05) < 0.05, "starts slowly");
        assert!(c.evaluate(0.95) > 0.95, "ends slowly");
    }

    #[test]
    fn clamped_auto_does_not_overshoot_a_peak() {
        // A torque curve: rises to a peak at 4500 rpm and falls off.
        let torque = AnimationCurve::smooth(&[(1000.0, 200.0), (4500.0, 400.0), (7000.0, 300.0)]);
        assert_eq!(torque.keys[1].in_tangent, 0.0, "flat at the peak");
        let max = (1000..=7000)
            .step_by(50)
            .map(|r| torque.evaluate(r as f32))
            .fold(f32::MIN, f32::max);
        assert!(
            (max - 400.0).abs() < 1e-3,
            "never above the peak key: {max}"
        );
        assert!(torque.evaluate(3000.0) > 300.0 && torque.evaluate(3000.0) < 400.0);
    }

    #[test]
    fn wrap_modes() {
        let mut c = AnimationCurve::linear(0.0, 0.0, 1.0, 1.0);
        c.post_wrap_mode = CurveWrapMode::Loop;
        assert!((c.evaluate(1.25) - 0.25).abs() < 1e-5);
        c.post_wrap_mode = CurveWrapMode::PingPong;
        assert!((c.evaluate(1.25) - 0.75).abs() < 1e-5);
        c.pre_wrap_mode = CurveWrapMode::Loop;
        assert!((c.evaluate(-0.25) - 0.75).abs() < 1e-5);
    }

    #[test]
    fn key_editing_keeps_order() {
        let mut c = AnimationCurve::linear(0.0, 0.0, 1.0, 1.0);
        let i = c.add_key_at(0.5);
        assert_eq!(i, 1);
        assert!((c.keys[1].value - 0.5).abs() < 1e-5);
        let j = c.move_key(1, 2.0, 3.0);
        assert_eq!(j, 2, "moved past the last key");
        assert_eq!(c.time_range(), (0.0, 2.0));
        c.remove_key(0);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn gradient_blend_and_fixed() {
        let mut g = Gradient::two(
            Color::new(1.0, 0.0, 0.0, 1.0),
            Color::new(0.0, 0.0, 1.0, 0.0),
        );
        let mid = g.evaluate(0.5);
        assert!((mid.r - 0.5).abs() < 1e-5 && (mid.b - 0.5).abs() < 1e-5);
        assert!((mid.a - 0.5).abs() < 1e-5);
        g.mode = GradientMode::Fixed;
        assert_eq!(g.evaluate(0.5).b, 1.0, "fixed mode jumps to the next key");
        assert_eq!(g.evaluate(0.0).r, 1.0);
    }

    #[test]
    fn min_max_curve_modes() {
        assert_eq!(MinMaxCurve::constant(2.0).evaluate(0.3, 0.9), 2.0);
        assert_eq!(MinMaxCurve::random(1.0, 3.0).evaluate(0.0, 0.5), 2.0);
        let c = MinMaxCurve::curve(10.0, AnimationCurve::linear(0.0, 1.0, 1.0, 0.0));
        assert!((c.evaluate(0.25, 0.0) - 7.5).abs() < 1e-4);
        assert!((c.max_value() - 10.0).abs() < 1e-4);
    }

    #[test]
    fn curves_round_trip_through_values() {
        let mut c = AnimationCurve::smooth(&[(0.0, 1.0), (0.5, 2.0), (1.0, 0.0)]);
        c.post_wrap_mode = CurveWrapMode::Loop;
        let v = c.to_value();
        assert!(matches!(v, Value::Object(_)));
        assert_eq!(AnimationCurve::from_value(v), Some(c));
        let g = MinMaxGradient::gradient(Gradient::two(Color::RED, Color::BLUE));
        assert_eq!(MinMaxGradient::from_value(g.to_value()), Some(g));
    }
}
