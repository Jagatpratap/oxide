//! [`Ctx`]: what a component sees while one of its messages runs — the Unity equivalents of
//! `this.gameObject`, `this.transform`, `GetComponent`, `Instantiate`, `Destroy`, `Time`
//! and `Input`.

use crate::component::Component;
use crate::id::{ComponentId, GameObjectId};
use crate::world::{GameObject, Space, World};
use oxide_assets::AssetServer;
use oxide_core::Time;
use oxide_input::Input;
use oxide_math::{Quaternion, QuaternionExt, Vector3};

pub struct Ctx<'w> {
    pub world: &'w mut World,
    /// The GameObject this component is on (Unity `gameObject`).
    pub this: GameObjectId,
    /// This component instance.
    pub component: ComponentId,
}

impl<'w> Ctx<'w> {
    pub fn game_object(&self) -> &GameObject {
        self.world.get(self.this).expect("ctx object is alive")
    }

    pub fn game_object_mut(&mut self) -> &mut GameObject {
        self.world.get_mut(self.this).expect("ctx object is alive")
    }

    pub fn name(&self) -> &str {
        &self.game_object().name
    }

    /// Unity `transform` of this object.
    pub fn transform(&mut self) -> TransformRef<'_> {
        TransformRef {
            world: self.world,
            id: self.this,
        }
    }

    /// Transform of any object.
    pub fn transform_of(&mut self, id: GameObjectId) -> TransformRef<'_> {
        TransformRef {
            world: self.world,
            id,
        }
    }

    pub fn time(&self) -> &Time {
        self.world.resource::<Time>()
    }

    /// Shorthand for `Time.deltaTime` (the fixed step inside `fixed_update`).
    pub fn delta_time(&self) -> f32 {
        self.time().delta_time()
    }

    pub fn input(&self) -> &Input {
        self.world.resource::<Input>()
    }

    pub fn assets(&self) -> &AssetServer {
        self.world.resource::<AssetServer>()
    }

    pub fn assets_mut(&mut self) -> &mut AssetServer {
        self.world.resource_mut::<AssetServer>()
    }

    pub fn resource<T: 'static>(&self) -> &T {
        self.world.resource::<T>()
    }

    pub fn resource_mut<T: 'static>(&mut self) -> &mut T {
        self.world.resource_mut::<T>()
    }

    /// Unity `GetComponent<T>()` on this object. (A component cannot get itself while its
    /// own message is running.)
    pub fn get_component<T: Component>(&self) -> Option<&T> {
        self.world.get_component::<T>(self.this)
    }

    pub fn get_component_mut<T: Component>(&mut self) -> Option<&mut T> {
        self.world.get_component_mut::<T>(self.this)
    }

    pub fn get_component_in_children<T: Component>(&self) -> Option<(GameObjectId, &T)> {
        self.world.get_component_in_children::<T>(self.this)
    }

    pub fn get_component_in_parent<T: Component>(&self) -> Option<(GameObjectId, &T)> {
        self.world.get_component_in_parent::<T>(self.this)
    }

    pub fn add_component<T: Component>(&mut self, c: T) -> Option<ComponentId> {
        self.world.add_component(self.this, c)
    }

    /// Unity `Instantiate(original)`.
    pub fn instantiate(&mut self, original: GameObjectId) -> Option<GameObjectId> {
        self.world.instantiate(original)
    }

    /// Unity `Instantiate(original, position, rotation)`.
    pub fn instantiate_at(
        &mut self,
        original: GameObjectId,
        position: Vector3,
        rotation: Quaternion,
    ) -> Option<GameObjectId> {
        let id = self.world.instantiate(original)?;
        self.world.set_position(id, position);
        self.world.set_rotation(id, rotation);
        Some(id)
    }

    /// Unity `Instantiate(prefab)` for a prefab file (`Assets/Bullet.prefab`). Errors are
    /// logged and give `None`.
    pub fn instantiate_prefab(&mut self, path: &str) -> Option<GameObjectId> {
        match self.world.instantiate_prefab(path, None) {
            Ok(id) => Some(id),
            Err(e) => {
                oxide_core::debug_log_error!("{e}");
                None
            }
        }
    }

    /// Unity `Instantiate(prefab, position, rotation)`.
    pub fn instantiate_prefab_at(
        &mut self,
        path: &str,
        position: Vector3,
        rotation: Quaternion,
    ) -> Option<GameObjectId> {
        let id = self.instantiate_prefab(path)?;
        self.world.set_position(id, position);
        self.world.set_rotation(id, rotation);
        Some(id)
    }

    /// Unity `SceneManager.LoadScene`: a scene path (`Assets/Scenes/Level2.scene`), name
    /// (`Level2`) or build index (`"1"`). Happens at the end of this frame.
    pub fn load_scene(&mut self, scene: &str) {
        if let Some(app) = self.world.try_resource_mut::<crate::Application>() {
            app.pending_scene = Some(scene.to_owned());
        }
    }

    /// Unity `SceneManager.LoadSceneAsync`: loads a scene (path, or name of a build scene)
    /// on a background thread. `additive` adds it to the open scenes (`LoadSceneMode.Additive`);
    /// otherwise it replaces them. Poll with [`scene_operation`](Self::scene_operation).
    pub fn load_scene_async(&mut self, scene: &str, additive: bool) -> crate::AsyncOperation {
        let path = self
            .world
            .try_resource::<crate::Application>()
            .and_then(|a| a.resolve_scene(scene, &[]))
            .unwrap_or_else(|| scene.to_owned());
        self.world.load_scene_async(&path, additive)
    }

    /// Unity `SceneManager.UnloadSceneAsync`: unloads an additive scene at the end of the frame.
    pub fn unload_scene_async(&mut self, scene: crate::SceneHandle) -> crate::AsyncOperation {
        self.world.unload_scene_async(scene)
    }

    /// Progress of a scene load or unload (Unity `AsyncOperation.progress` / `isDone`).
    pub fn scene_operation(&self, op: crate::AsyncOperation) -> Option<crate::SceneOpStatus> {
        self.world.scene_operation(op)
    }

    /// Unity `Application.Quit()` (ignored in the editor's Play mode, like Unity).
    pub fn quit(&mut self) {
        if let Some(app) = self.world.try_resource_mut::<crate::Application>() {
            app.quit = true;
        }
    }

    /// Unity `Destroy(obj)`.
    pub fn destroy(&mut self, id: GameObjectId) {
        self.world.destroy(id);
    }

    /// Unity `Destroy(obj, t)`: destroys after `seconds` of game time.
    pub fn destroy_after(&mut self, id: GameObjectId, seconds: f32) {
        let at = self.time().time() + seconds;
        if let Some(t) = self.world.try_resource_mut::<crate::timers::Timers>() {
            t.destroy_at(id, at);
        }
    }

    pub fn destroy_self(&mut self) {
        self.world.destroy(self.this);
    }

    /// Unity `Destroy(this)` — removes just this component.
    pub fn destroy_this_component(&mut self) {
        self.world.destroy_component(self.this, self.component);
    }

    /// Unity `enabled = value` for this component.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.world
            .set_component_enabled(self.this, self.component, enabled);
    }

    pub fn set_active(&mut self, active: bool) {
        self.world.set_active(self.this, active);
    }

    pub fn find(&self, name: &str) -> Option<GameObjectId> {
        self.world.find(name)
    }

    pub fn find_with_tag(&self, tag: &str) -> Vec<GameObjectId> {
        self.world.find_with_tag(tag)
    }

    pub fn compare_tag(&self, tag: &str) -> bool {
        self.game_object().tag == tag
    }

    /// Runs `f` on component `T` of another object with that object's context — the safe
    /// way to call into another script (Unity: `other.GetComponent<T>().DoThing()`).
    pub fn with_component<T: Component, R>(
        &mut self,
        id: GameObjectId,
        f: impl FnOnce(&mut T, &mut Ctx) -> R,
    ) -> Option<R> {
        let cid = self.world.component_id::<T>(id)?;
        let mut out = None;
        self.world.call(id, cid, |c, ctx| {
            if let Some(t) = crate::component::downcast_mut::<T>(c) {
                out = Some(f(t, ctx));
            }
        });
        out
    }

    /// Unity `SendMessage`.
    pub fn send_message(
        &mut self,
        id: GameObjectId,
        message: &str,
        arg: oxide_core::reflect::Value,
    ) -> bool {
        self.world.send_message(id, message, arg)
    }

    /// Unity `Debug.DrawLine` (visible for one frame).
    pub fn draw_line(&mut self, a: Vector3, b: Vector3, color: oxide_math::Color) {
        if let Some(d) = self.world.try_resource_mut::<crate::system::DebugDraw>() {
            d.line(a, b, color);
        }
    }

    /// Unity `Debug.DrawRay`.
    pub fn draw_ray(&mut self, origin: Vector3, dir: Vector3, color: oxide_math::Color) {
        self.draw_line(origin, origin + dir, color);
    }

    /// Unity `Invoke`: calls `on_message(name)` on this object after `seconds`.
    pub fn invoke(&mut self, message: &str, seconds: f32) {
        let at = self.time().time() + seconds;
        let id = self.this;
        if let Some(t) = self.world.try_resource_mut::<crate::timers::Timers>() {
            t.message_at(id, message, at);
        }
    }
}

/// Unity `Transform` API for one object.
pub struct TransformRef<'a> {
    pub world: &'a mut World,
    pub id: GameObjectId,
}

impl TransformRef<'_> {
    pub fn position(&self) -> Vector3 {
        self.world.position(self.id)
    }
    pub fn set_position(&mut self, p: Vector3) {
        self.world.set_position(self.id, p)
    }
    pub fn rotation(&self) -> Quaternion {
        self.world.rotation(self.id)
    }
    pub fn set_rotation(&mut self, q: Quaternion) {
        self.world.set_rotation(self.id, q)
    }
    pub fn euler_angles(&self) -> Vector3 {
        self.world.euler_angles(self.id)
    }
    pub fn set_euler_angles(&mut self, e: Vector3) {
        self.world.set_euler_angles(self.id, e)
    }
    pub fn local_position(&self) -> Vector3 {
        self.world
            .transform(self.id)
            .map_or(Vector3::ZERO, |t| t.local_position)
    }
    pub fn set_local_position(&mut self, p: Vector3) {
        if let Some(t) = self.world.transform_mut(self.id) {
            t.local_position = p;
        }
    }
    pub fn local_rotation(&self) -> Quaternion {
        self.world
            .transform(self.id)
            .map_or(Quaternion::IDENTITY, |t| t.local_rotation)
    }
    pub fn set_local_rotation(&mut self, q: Quaternion) {
        if let Some(t) = self.world.transform_mut(self.id) {
            t.local_rotation = q.normalize();
        }
    }
    pub fn local_euler_angles(&self) -> Vector3 {
        self.local_rotation().euler_angles()
    }
    pub fn set_local_euler_angles(&mut self, e: Vector3) {
        self.set_local_rotation(Quaternion::euler_v(e))
    }
    pub fn local_scale(&self) -> Vector3 {
        self.world
            .transform(self.id)
            .map_or(Vector3::ONE, |t| t.local_scale)
    }
    pub fn set_local_scale(&mut self, s: Vector3) {
        if let Some(t) = self.world.transform_mut(self.id) {
            t.local_scale = s;
        }
    }
    pub fn lossy_scale(&self) -> Vector3 {
        self.world.lossy_scale(self.id)
    }
    pub fn forward(&self) -> Vector3 {
        self.world.forward(self.id)
    }
    pub fn right(&self) -> Vector3 {
        self.world.right(self.id)
    }
    pub fn up(&self) -> Vector3 {
        self.world.up(self.id)
    }
    /// Unity `Translate(delta)` — in local space by default.
    pub fn translate(&mut self, delta: Vector3) {
        self.world.translate(self.id, delta, Space::Self_)
    }
    pub fn translate_world(&mut self, delta: Vector3) {
        self.world.translate(self.id, delta, Space::World)
    }
    /// Unity `Rotate(eulers)` — local space.
    pub fn rotate(&mut self, euler: Vector3) {
        self.world.rotate(self.id, euler, Space::Self_)
    }
    pub fn rotate_world(&mut self, euler: Vector3) {
        self.world.rotate(self.id, euler, Space::World)
    }
    pub fn rotate_around(&mut self, point: Vector3, axis: Vector3, angle: f32) {
        self.world.rotate_around(self.id, point, axis, angle)
    }
    pub fn look_at(&mut self, target: Vector3) {
        self.world.look_at(self.id, target, Vector3::Y)
    }
    pub fn transform_point(&self, p: Vector3) -> Vector3 {
        self.world.transform_point(self.id, p)
    }
    pub fn transform_direction(&self, d: Vector3) -> Vector3 {
        self.world.transform_direction(self.id, d)
    }
    pub fn inverse_transform_point(&self, p: Vector3) -> Vector3 {
        self.world.inverse_transform_point(self.id, p)
    }
    pub fn inverse_transform_direction(&self, d: Vector3) -> Vector3 {
        self.world.inverse_transform_direction(self.id, d)
    }
    pub fn parent(&self) -> Option<GameObjectId> {
        self.world.parent(self.id)
    }
    pub fn set_parent(&mut self, parent: Option<GameObjectId>, world_position_stays: bool) {
        let _ = self.world.set_parent(self.id, parent, world_position_stays);
    }
    pub fn child_count(&self) -> usize {
        self.world.children(self.id).len()
    }
    pub fn get_child(&self, i: usize) -> Option<GameObjectId> {
        self.world.children(self.id).get(i).copied()
    }
    pub fn find(&self, path: &str) -> Option<GameObjectId> {
        self.world.find_child(self.id, path)
    }
}
