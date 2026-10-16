//! Racing AI (Unity Standard Assets' `WaypointCircuit` + `CarAIControl`): a closed route
//! through waypoint objects, and a driver that follows it with a [`CarController`].

use crate::car::CarController;
use oxide_core::Reflect;
use oxide_math::{Color, Vector3};
use oxide_scene::{Component, Ctx, GameObjectId, Gizmos, World};

/// A closed route through waypoints (`waypoints`, or the object's children in order).
#[derive(Clone, Debug, Reflect)]
pub struct WaypointCircuit {
    pub waypoints: Vec<GameObjectId>,
    /// Smooth the route through the points (Catmull-Rom) instead of straight segments.
    pub smooth: bool,
}

impl Default for WaypointCircuit {
    fn default() -> Self {
        Self {
            waypoints: Vec::new(),
            smooth: true,
        }
    }
}

/// Route sampled from a circuit: points and cumulative distances (closed loop).
pub struct Route {
    points: Vec<Vector3>,
    dist: Vec<f32>,
    pub length: f32,
    smooth: bool,
}

impl Route {
    pub fn new(points: Vec<Vector3>, smooth: bool) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        let mut dist = Vec::with_capacity(points.len() + 1);
        let mut d = 0.0;
        for i in 0..points.len() {
            dist.push(d);
            d += (points[(i + 1) % points.len()] - points[i]).length();
        }
        dist.push(d);
        Some(Self {
            points,
            dist,
            length: d.max(1e-3),
            smooth,
        })
    }

    /// Position on the route `d` meters from the start (wraps around).
    pub fn point(&self, d: f32) -> Vector3 {
        let d = d.rem_euclid(self.length);
        let n = self.points.len();
        let i = self
            .dist
            .partition_point(|&x| x <= d)
            .saturating_sub(1)
            .min(n - 1);
        let seg = (self.dist[i + 1] - self.dist[i]).max(1e-4);
        let t = (d - self.dist[i]) / seg;
        let p = |k: isize| self.points[(i as isize + k).rem_euclid(n as isize) as usize];
        if !self.smooth || n < 3 {
            return p(0).lerp(p(1), t);
        }
        let (p0, p1, p2, p3) = (p(-1), p(0), p(1), p(2));
        let t2 = t * t;
        let t3 = t2 * t;
        0.5 * (2.0 * p1
            + (p2 - p0) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
            + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
    }

    /// Unit direction of travel at `d`.
    pub fn direction(&self, d: f32) -> Vector3 {
        (self.point(d + 0.5) - self.point(d - 0.5)).normalize_or(Vector3::Z)
    }

    /// Route distance of the point closest to `p` (coarse search).
    pub fn closest(&self, p: Vector3) -> f32 {
        let steps = (self.length / 1.0).ceil().clamp(16.0, 4000.0) as usize;
        (0..steps)
            .map(|k| k as f32 * self.length / steps as f32)
            .min_by(|a, b| {
                (self.point(*a) - p)
                    .length_squared()
                    .total_cmp(&(self.point(*b) - p).length_squared())
            })
            .unwrap_or(0.0)
    }
}

impl WaypointCircuit {
    /// The route through this circuit's waypoints as they are now.
    pub fn route(&self, world: &World, id: GameObjectId) -> Option<Route> {
        let ids: Vec<GameObjectId> = if self.waypoints.is_empty() {
            world.children(id).to_vec()
        } else {
            self.waypoints.clone()
        };
        let pts = ids
            .into_iter()
            .filter(|&w| world.is_alive(w))
            .map(|w| world.position(w))
            .collect();
        Route::new(pts, self.smooth)
    }
}

impl Component for WaypointCircuit {
    fn on_draw_gizmos(&self, world: &World, id: GameObjectId, g: &mut Gizmos) {
        let Some(r) = self.route(world, id) else {
            return;
        };
        g.color = Color::YELLOW;
        let steps = (r.length / 2.0).clamp(16.0, 2000.0) as usize;
        let mut prev = r.point(0.0);
        for k in 1..=steps {
            let p = r.point(k as f32 * r.length / steps as f32);
            g.draw_line(prev, p);
            prev = p;
        }
        for &p in &r.points {
            g.draw_wire_sphere(p, 0.6);
        }
    }
}

/// Drives a [`CarController`] around a [`WaypointCircuit`]: steers toward a point ahead on
/// the route, slows for corners, and backs out when stuck.
#[derive(Clone, Debug, Reflect)]
pub struct CarAIControl {
    /// Object with the WaypointCircuit to follow.
    pub circuit: Option<GameObjectId>,
    /// How far ahead to aim: `lookahead + speed (m/s) × lookahead_per_speed` meters.
    pub lookahead: f32,
    pub lookahead_per_speed: f32,
    /// Fraction of top speed kept in the sharpest corners.
    #[reflect(range(0, 1))]
    pub cautious_speed_factor: f32,
    /// Corner angle (degrees) at which the car is fully cautious.
    pub cautious_max_angle: f32,
    pub steer_sensitivity: f32,
    pub accel_sensitivity: f32,
    /// Top speed this driver aims for (km/h).
    pub top_speed_kmh: f32,
    /// Distance driven along the route (laps × length + position).
    #[reflect(skip)]
    pub progress: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub started: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub stuck_time: f32,
    #[reflect(skip)]
    #[doc(hidden)]
    pub reverse_time: f32,
}

impl Default for CarAIControl {
    fn default() -> Self {
        Self {
            circuit: None,
            lookahead: 6.0,
            lookahead_per_speed: 0.6,
            cautious_speed_factor: 0.45,
            cautious_max_angle: 50.0,
            steer_sensitivity: 0.04,
            accel_sensitivity: 0.08,
            top_speed_kmh: 120.0,
            progress: 0.0,
            started: false,
            stuck_time: 0.0,
            reverse_time: 0.0,
        }
    }
}

impl Component for CarAIControl {
    fn fixed_update(&mut self, ctx: &mut Ctx) {
        let me = ctx.this;
        let Some(cid) = self.circuit else { return };
        let route = {
            let w = &*ctx.world;
            let Some(c) = w.get_component::<WaypointCircuit>(cid) else {
                return;
            };
            match c.route(w, cid) {
                Some(r) => r,
                None => return,
            }
        };
        let pos = ctx.world.position(me);
        let fwd = ctx.world.forward(me);
        if !self.started {
            self.progress = route.closest(pos);
            self.started = true;
        }
        // Advance progress as the car passes the progress point.
        let pp = route.point(self.progress);
        let ahead = (pos - pp).dot(route.direction(self.progress));
        if ahead > 0.0 {
            self.progress += ahead;
        }
        let Some(car) = ctx.world.get_component::<CarController>(me) else {
            return;
        };
        let speed_kmh = car.speed_kmh();
        let speed = speed_kmh / 3.6;
        let dt = ctx.delta_time();

        let target_d = self.progress + self.lookahead + speed * self.lookahead_per_speed;
        let target = route.point(target_d);
        let local = ctx.world.inverse_transform_point(me, target);
        let angle = local.x.atan2(local.z).to_degrees();
        let corner = route
            .direction(self.progress)
            .angle_between(route.direction(target_d + 10.0))
            .to_degrees();
        let caution = (corner / self.cautious_max_angle.max(1.0)).clamp(0.0, 1.0);
        let desired = self.top_speed_kmh * (1.0 - caution * (1.0 - self.cautious_speed_factor));
        let mut throttle = ((desired - speed_kmh) * self.accel_sensitivity).clamp(-1.0, 1.0);
        let mut steer = (angle * self.steer_sensitivity).clamp(-1.0, 1.0);

        // Stuck (pushing but not moving): reverse out with opposite lock.
        if self.reverse_time > 0.0 {
            self.reverse_time -= dt;
            throttle = -1.0;
            steer = -steer.signum();
        } else if throttle > 0.3 && speed_kmh.abs() < 2.0 {
            self.stuck_time += dt;
            if self.stuck_time > 2.5 {
                self.reverse_time = 1.5;
                self.stuck_time = 0.0;
            }
        } else {
            self.stuck_time = 0.0;
        }
        let _ = fwd;
        if let Some(car) = ctx.world.get_component_mut::<CarController>(me) {
            car.use_input = false;
            car.throttle_input = throttle;
            car.steer_input = steer;
            car.handbrake_input = false;
        }
    }

    fn on_draw_gizmos(&self, world: &World, id: GameObjectId, g: &mut Gizmos) {
        let Some(cid) = self.circuit else { return };
        let Some(r) = world
            .get_component::<WaypointCircuit>(cid)
            .and_then(|c| c.route(world, cid))
        else {
            return;
        };
        g.color = Color::CYAN;
        g.draw_line(world.position(id), r.point(self.progress + self.lookahead));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_wraps_and_smooths_through_points() {
        let pts = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 10.0),
            Vector3::new(0.0, 0.0, 10.0),
        ];
        let r = Route::new(pts.clone(), true).unwrap();
        assert!((r.length - 40.0).abs() < 1e-4);
        for (i, p) in pts.iter().enumerate() {
            assert!(
                (r.point(i as f32 * 10.0) - *p).length() < 1e-3,
                "passes through {i}"
            );
        }
        assert!((r.point(45.0) - r.point(5.0)).length() < 1e-4, "wraps");
        assert!((r.closest(Vector3::new(10.5, 0.0, 5.0)) - 15.0).abs() < 1.5);
        assert!(r.direction(5.0).x > 0.9);
    }
}
