//! Unity `Color` / `Color32`. Reference: <https://docs.unity3d.com/ScriptReference/Color.html>

use crate::{Mathf, Vector4};
use serde::{Deserialize, Serialize};

/// RGBA color with `f32` components, normally in `[0, 1]` (HDR values may exceed 1).
#[repr(C)]
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    bytemuck::Pod,
    bytemuck::Zeroable,
)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const RED: Self = Self::new(1.0, 0.0, 0.0, 1.0);
    pub const GREEN: Self = Self::new(0.0, 1.0, 0.0, 1.0);
    pub const BLUE: Self = Self::new(0.0, 0.0, 1.0, 1.0);
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    pub const YELLOW: Self = Self::new(1.0, 0.921_568_6, 0.015_686_275, 1.0);
    pub const CYAN: Self = Self::new(0.0, 1.0, 1.0, 1.0);
    pub const MAGENTA: Self = Self::new(1.0, 0.0, 1.0, 1.0);
    pub const GRAY: Self = Self::new(0.5, 0.5, 0.5, 1.0);
    pub const GREY: Self = Self::GRAY;
    pub const CLEAR: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::new(r, g, b, 1.0)
    }

    /// Interpolates with `t` clamped to `[0, 1]`.
    pub fn lerp(a: Self, b: Self, t: f32) -> Self {
        Self::lerp_unclamped(a, b, Mathf::clamp01(t))
    }

    pub fn lerp_unclamped(a: Self, b: Self, t: f32) -> Self {
        Self::from(Vector4::from(a).lerp(Vector4::from(b), t))
    }

    /// This color (assumed sRGB/gamma) converted to linear space. Alpha is unchanged.
    pub fn linear(self) -> Self {
        let f = Mathf::gamma_to_linear_space;
        Self::new(f(self.r), f(self.g), f(self.b), self.a)
    }

    /// This color (assumed linear) converted to sRGB/gamma space. Alpha is unchanged.
    pub fn gamma(self) -> Self {
        let f = Mathf::linear_to_gamma_space;
        Self::new(f(self.r), f(self.g), f(self.b), self.a)
    }

    pub fn grayscale(self) -> f32 {
        0.299 * self.r + 0.587 * self.g + 0.114 * self.b
    }

    pub fn max_color_component(self) -> f32 {
        self.r.max(self.g).max(self.b)
    }

    /// Unity `Color.RGBToHSV`: returns `(h, s, v)` each in `[0, 1]`.
    pub fn rgb_to_hsv(self) -> (f32, f32, f32) {
        let max = self.max_color_component();
        let min = self.r.min(self.g).min(self.b);
        let delta = max - min;
        if max <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        if delta == 0.0 {
            return (0.0, 0.0, max);
        }
        let mut h = if max == self.r {
            (self.g - self.b) / delta
        } else if max == self.g {
            2.0 + (self.b - self.r) / delta
        } else {
            4.0 + (self.r - self.g) / delta
        } / 6.0;
        if h < 0.0 {
            h += 1.0;
        }
        (h, delta / max, max)
    }

    /// Unity `Color.HSVToRGB` (non-HDR: result is clamped to `[0, 1]`).
    pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Self {
        if s == 0.0 {
            return Self::rgb(v, v, v);
        }
        let h6 = Mathf::repeat(h, 1.0) * 6.0;
        let i = h6.floor();
        let f = h6 - i;
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));
        let (r, g, b) = match i as i32 % 6 {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        Self::rgb(Mathf::clamp01(r), Mathf::clamp01(g), Mathf::clamp01(b))
    }
}

impl From<Color> for Vector4 {
    fn from(c: Color) -> Self {
        Vector4::new(c.r, c.g, c.b, c.a)
    }
}

impl From<Vector4> for Color {
    fn from(v: Vector4) -> Self {
        Color::new(v.x, v.y, v.z, v.w)
    }
}

impl From<Color> for [f32; 4] {
    fn from(c: Color) -> Self {
        [c.r, c.g, c.b, c.a]
    }
}

impl std::ops::Mul<f32> for Color {
    type Output = Color;
    fn mul(self, k: f32) -> Color {
        Color::new(self.r * k, self.g * k, self.b * k, self.a * k)
    }
}

impl std::ops::Mul for Color {
    type Output = Color;
    fn mul(self, o: Color) -> Color {
        Color::new(self.r * o.r, self.g * o.g, self.b * o.b, self.a * o.a)
    }
}

impl std::ops::Add for Color {
    type Output = Color;
    fn add(self, o: Color) -> Color {
        Color::new(self.r + o.r, self.g + o.g, self.b + o.b, self.a + o.a)
    }
}

/// 8-bit-per-channel RGBA color (Unity `Color32`).
#[repr(C)]
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    bytemuck::Pod,
    bytemuck::Zeroable,
)]
pub struct Color32 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color32 {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

impl From<Color> for Color32 {
    fn from(c: Color) -> Self {
        let q = |x: f32| (Mathf::clamp01(x) * 255.0).round() as u8;
        Color32::new(q(c.r), q(c.g), q(c.b), q(c.a))
    }
}

impl From<Color32> for Color {
    fn from(c: Color32) -> Self {
        Color::new(
            c.r as f32 / 255.0,
            c.g as f32 / 255.0,
            c.b as f32 / 255.0,
            c.a as f32 / 255.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip() {
        let c = Color::rgb(0.2, 0.6, 0.9);
        let (h, s, v) = c.rgb_to_hsv();
        let back = Color::hsv_to_rgb(h, s, v);
        assert!((Vector4::from(c) - Vector4::from(back)).length() < 1e-5);
    }

    #[test]
    fn color32_round_trip() {
        let c = Color32::new(12, 200, 99, 255);
        assert_eq!(Color32::from(Color::from(c)), c);
    }
}
