//! Unity's `WindZone`: wind that bends terrain trees and grass and pushes particles (with
//! External Forces on). Directional zones blow everywhere along the object's forward;
//! spherical zones blow outward within a radius, fading to the edge.
//! Reference: <https://docs.unity3d.com/Manual/class-WindZone.html>

use crate::component::Component;
use crate::world::World;
use oxide_core::Reflect;
use oxide_math::Vector3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum WindZoneMode {
    #[default]
    Directional,
    Spherical,
}

/// Unity `WindZone`.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct WindZone {
    pub mode: WindZoneMode,
    /// Spherical: how far the wind reaches.
    pub radius: f32,
    /// Base strength.
    pub wind_main: f32,
    /// Random gusts on top.
    pub wind_turbulence: f32,
    /// How much the strength rises and falls, and how often (per second).
    pub wind_pulse_magnitude: f32,
    pub wind_pulse_frequency: f32,
}

impl Default for WindZone {
    fn default() -> Self {
        Self {
            mode: WindZoneMode::Directional,
            radius: 20.0,
            wind_main: 1.0,
            wind_turbulence: 1.0,
            wind_pulse_magnitude: 0.5,
            wind_pulse_frequency: 0.01,
        }
    }
}

impl Component for WindZone {
    fn allow_multiple(&self) -> bool {
        false
    }
}

fn hash(x: f32) -> f32 {
    let v = (x * 12.9898).sin() * 43758.547;
    v - v.floor()
}

/// Smooth 1D noise in -1..1.
fn noise(t: f32) -> f32 {
    let i = t.floor();
    let f = t - i;
    let s = f * f * (3.0 - 2.0 * f);
    (hash(i) + (hash(i + 1.0) - hash(i)) * s) * 2.0 - 1.0
}

impl WindZone {
    /// Strength now: main, pulsing, plus turbulence varying over time and space.
    pub fn strength(&self, time: f32, at: Vector3) -> f32 {
        let pulse = self.wind_pulse_magnitude
            * (time * self.wind_pulse_frequency.max(0.0) * std::f32::consts::TAU).sin();
        let gust = self.wind_turbulence * noise(time * 1.3 + (at.x + at.z) * 0.07) * 0.5;
        (self.wind_main * (1.0 + pulse) + gust).max(0.0)
    }
}

/// An active Wind Zone with its placement (see [`wind_zones`]).
#[derive(Clone, Debug)]
pub struct PlacedWindZone {
    pub zone: WindZone,
    pub position: Vector3,
    pub forward: Vector3,
}

/// Every active Wind Zone, for evaluating wind without the World (see [`wind_from`]).
pub fn wind_zones(world: &World) -> Vec<PlacedWindZone> {
    world
        .query::<WindZone>()
        .filter(|(id, _)| world.get(*id).is_some_and(|o| o.active_in_hierarchy()))
        .map(|(id, z)| PlacedWindZone {
            zone: z.clone(),
            position: world.position(id),
            forward: world.rotation(id) * Vector3::Z,
        })
        .collect()
}

/// The wind (direction × strength) at `position` from `zones`.
pub fn wind_from(zones: &[PlacedWindZone], position: Vector3, time: f32) -> Vector3 {
    let mut out = Vector3::ZERO;
    for z in zones {
        let s = z.zone.strength(time, position);
        match z.zone.mode {
            WindZoneMode::Directional => out += z.forward * s,
            WindZoneMode::Spherical => {
                let d = position - z.position;
                let r = z.zone.radius.max(1e-4);
                let dist = d.length();
                if dist < r {
                    out += d.normalize_or(Vector3::Y) * s * (1.0 - dist / r);
                }
            }
        }
    }
    out
}

/// The wind (direction × strength) at `position`, summed over every active Wind Zone.
pub fn wind_at(world: &World, position: Vector3, time: f32) -> Vector3 {
    wind_from(&wind_zones(world), position, time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_math::Quaternion;

    #[test]
    fn directional_and_spherical_zones() {
        let mut w = World::new();
        let d = w.create_at(
            "Wind",
            Vector3::ZERO,
            Quaternion::from_rotation_y(std::f32::consts::FRAC_PI_2),
        );
        w.add_component(
            d,
            WindZone {
                wind_turbulence: 0.0,
                wind_pulse_magnitude: 0.0,
                wind_main: 2.0,
                ..Default::default()
            },
        );
        let v = wind_at(&w, Vector3::new(100.0, 0.0, 0.0), 1.0);
        assert!(
            (v - Vector3::new(2.0, 0.0, 0.0)).length() < 1e-4,
            "blows along +X: {v}"
        );
        let s = w.create_at("Blast", Vector3::new(0.0, 0.0, 50.0), Quaternion::IDENTITY);
        w.add_component(
            s,
            WindZone {
                mode: WindZoneMode::Spherical,
                radius: 10.0,
                wind_main: 4.0,
                wind_turbulence: 0.0,
                wind_pulse_magnitude: 0.0,
                ..Default::default()
            },
        );
        let near = wind_at(&w, Vector3::new(0.0, 0.0, 55.0), 1.0) - Vector3::new(2.0, 0.0, 0.0);
        assert!(
            (near - Vector3::new(0.0, 0.0, 2.0)).length() < 1e-4,
            "outward, half strength: {near}"
        );
        let far = wind_at(&w, Vector3::new(0.0, 0.0, 70.0), 1.0);
        assert!(
            (far - Vector3::new(2.0, 0.0, 0.0)).length() < 1e-4,
            "outside the radius"
        );
        // Pulses vary the strength over time.
        let z = WindZone {
            wind_turbulence: 0.0,
            wind_pulse_magnitude: 0.5,
            wind_pulse_frequency: 1.0,
            ..Default::default()
        };
        assert!((z.strength(0.25, Vector3::ZERO) - 1.5).abs() < 1e-4);
        assert!((z.strength(0.75, Vector3::ZERO) - 0.5).abs() < 1e-4);
    }
}
