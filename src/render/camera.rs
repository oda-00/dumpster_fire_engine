//! Camera system using engine-native types (Arena, Handle, Id, Arc<str>).

use glam::{Mat4, Vec3};
use std::sync::Arc;
use thin_vec::ThinVec;
use winit::event::ElementState;
use winit::keyboard::KeyCode;

use crate::resource_manager::manager::{Arena, Handle, Id};

// ── Handle / Id types ──────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CameraTag;
pub type CameraHandle = Handle<CameraTag>;

pub struct CameraMarker;
pub type CameraId = Id<CameraMarker>;

// ── Camera modes ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CameraMode {
    Fly,
    Orbit { target: [f32; 3], distance: f32 },
}

// ── Camera ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Camera {
    pub id: CameraId,
    pub name: Arc<str>,
    pub mode: CameraMode,
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    pub fn new(id: CameraId, name: Arc<str>, position: [f32; 3], yaw: f32, pitch: f32) -> Self {
        Self {
            id,
            name,
            mode: CameraMode::Fly,
            position,
            yaw,
            pitch,
            fov: 45.0_f32.to_radians(),
            near: 0.1,
            far: 100.0,
        }
    }

    pub fn new_orbit(
        id: CameraId,
        name: Arc<str>,
        target: [f32; 3],
        distance: f32,
        yaw: f32,
        pitch: f32,
    ) -> Self {
        let position = Self::orbit_position(target, distance, yaw, pitch);
        Self {
            id,
            name,
            mode: CameraMode::Orbit { target, distance },
            position,
            yaw,
            pitch,
            fov: 45.0_f32.to_radians(),
            near: 0.1,
            far: 100.0,
        }
    }

    fn orbit_position(target: [f32; 3], distance: f32, yaw: f32, pitch: f32) -> [f32; 3] {
        let x = target[0] + distance * pitch.cos() * yaw.cos();
        let y = target[1] + distance * pitch.sin();
        let z = target[2] + distance * pitch.cos() * yaw.sin();
        [x, y, z]
    }

    pub fn update_orbit(&mut self) {
        if let CameraMode::Orbit { target, distance } = self.mode {
            self.position = Self::orbit_position(target, distance, self.yaw, self.pitch);
        }
    }

    pub fn view_matrix(&self) -> [f32; 16] {
        let pos = Vec3::new(self.position[0], self.position[1], self.position[2]);
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let forward = Vec3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize();
        let right = Vec3::new(-sin_yaw, 0.0, cos_yaw).normalize();
        let up = right.cross(forward).normalize();
        let view = Mat4::look_at_rh(pos, pos + forward, up);
        view.to_cols_array()
    }

    pub fn projection_matrix(&self, aspect: f32) -> [f32; 16] {
        let proj = Mat4::perspective_rh(self.fov, aspect, self.near, self.far);
        proj.to_cols_array()
    }

    pub fn view_projection_matrix(&self, aspect: f32) -> [f32; 16] {
        let view = Mat4::from_cols_array(&self.view_matrix());
        let proj = Mat4::from_cols_array(&self.projection_matrix(aspect));
        (proj * view).to_cols_array()
    }

    pub fn move_forward(&mut self, delta: f32) {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let dir = Vec3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize();
        self.position[0] += dir.x * delta;
        self.position[1] += dir.y * delta;
        self.position[2] += dir.z * delta;
    }

    pub fn move_right(&mut self, delta: f32) {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let dir = Vec3::new(-sin_yaw, 0.0, cos_yaw).normalize();
        self.position[0] += dir.x * delta;
        self.position[1] += dir.y * delta;
        self.position[2] += dir.z * delta;
    }

    pub fn move_up(&mut self, delta: f32) {
        self.position[1] += delta;
    }

    pub fn rotate(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw += delta_yaw;
        self.pitch = (self.pitch + delta_pitch).clamp(-1.5, 1.5);
        if let CameraMode::Orbit { .. } = self.mode {
            self.update_orbit();
        }
    }
}

// ── Camera arena ────────────────────────────────────────────────────────────

pub struct CameraArena {
    cameras: Arena<CameraTag, Camera>,
    cache: ThinVec<CameraHandle>,
}

impl CameraArena {
    pub fn new() -> Self {
        Self {
            cameras: Arena::new(),
            cache: ThinVec::new(),
        }
    }

    pub fn insert(&mut self, camera: Camera) -> CameraHandle {
        let handle = self.cameras.insert(camera);
        self.cache.push(handle);
        handle
    }

    pub fn get(&self, handle: CameraHandle) -> Option<&Camera> {
        self.cameras.get(handle)
    }

    pub fn get_mut(&mut self, handle: CameraHandle) -> Option<&mut Camera> {
        self.cameras.get_mut(handle)
    }

    pub fn remove(&mut self, handle: CameraHandle) -> Option<Camera> {
        let cam = self.cameras.remove(handle)?;
        if let Some(pos) = self.cache.iter().position(|&h| h == handle) {
            self.cache.swap_remove(pos);
        }
        Some(cam)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Camera> {
        self.cameras.values()
    }

    pub fn len(&self) -> usize {
        self.cameras.len()
    }
}

// ── Camera controller ───────────────────────────────────────────────────────

pub struct CameraController {
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    mouse_grabbed: bool,
    mouse_sensitivity: f32,
    move_speed: f32,
}

impl CameraController {
    pub fn new(move_speed: f32, mouse_sensitivity: f32) -> Self {
        Self {
            forward: false,
            backward: false,
            left: false,
            right: false,
            up: false,
            down: false,
            mouse_grabbed: false,
            move_speed,
            mouse_sensitivity,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode, state: ElementState) {
        let pressed = state == ElementState::Pressed;
        match key {
            KeyCode::KeyW => self.forward = pressed,
            KeyCode::KeyS => self.backward = pressed,
            KeyCode::KeyA => self.left = pressed,
            KeyCode::KeyD => self.right = pressed,
            KeyCode::KeyQ => self.up = pressed,
            KeyCode::KeyE => self.down = pressed,
            _ => {}
        }
    }

    pub fn handle_mouse(&mut self, dx: f32, dy: f32) -> (f32, f32) {
        if !self.mouse_grabbed {
            return (0.0, 0.0);
        }
        let yaw = -dx * self.mouse_sensitivity;
        let pitch = -dy * self.mouse_sensitivity;
        (yaw, pitch)
    }

    pub fn toggle_grab(&mut self) -> bool {
        self.mouse_grabbed = !self.mouse_grabbed;
        self.mouse_grabbed
    }

    pub fn set_grab(&mut self, grabbed: bool) {
        self.mouse_grabbed = grabbed;
    }

    pub fn is_grabbed(&self) -> bool {
        self.mouse_grabbed
    }

    pub fn update(&self, camera: &mut Camera, dt: f32) {
        let speed = self.move_speed * dt;
        if self.forward {
            camera.move_forward(speed);
        }
        if self.backward {
            camera.move_forward(-speed);
        }
        if self.right {
            camera.move_right(speed);
        }
        if self.left {
            camera.move_right(-speed);
        }
        if self.up {
            camera.move_up(speed);
        }
        if self.down {
            camera.move_up(-speed);
        }
    }
}
