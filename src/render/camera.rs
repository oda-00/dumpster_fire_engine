//! Camera system using engine-native types (Arena, Handle, Id, Arc<str>).

use glam::{Mat4, Quat, Vec3};
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

// ── Physical Lens Parameters ───────────────────────────────────────────────

/// Physical-lens parameters. `fov_y` is derived from focal_length and sensor_h.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lens {
    pub focal_length_mm: f32,  // e.g., 50.0
    pub sensor_w_mm:     f32,  // e.g., 36.0 (full-frame)
    pub sensor_h_mm:     f32,  // e.g., 24.0
}

impl Lens {
    pub fn fov_y(&self) -> f32 {
        2.0 * ((self.sensor_h_mm * 0.5) / self.focal_length_mm).atan()
    }
    pub const FULL_FRAME: Self = Lens {
        focal_length_mm: 50.0,
        sensor_w_mm: 36.0,
        sensor_h_mm: 24.0,
    };
}

// ── Exposure: Sensor + Shutter ────────────────────────────────────────────

/// Sensor + shutter parameters. EV15 (sunny-16) is calibration baseline.
/// Exposure multiplier = 2^(EV_REF - ev)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Exposure {
    pub aperture_fstop:  f32,  // e.g., 5.6
    pub shutter_seconds: f32,  // e.g., 1.0/60.0
    pub iso:             f32,  // e.g., 100.0
    pub ev_compensation: f32,  // user offset in stops; e.g., 0.0
}

impl Exposure {
    pub fn ev(&self) -> f32 {
        let n2 = self.aperture_fstop * self.aperture_fstop;
        (n2 / self.shutter_seconds).log2() - (self.iso / 100.0).log2() - self.ev_compensation
    }

    /// Scene-linear multiplier before tonemap.
    pub fn linear_scale(&self) -> f32 {
        const EV_REF: f32 = 15.0;
        2.0_f32.powf(EV_REF - self.ev())
    }
}

// ── Depth of Field ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dof {
    pub focus_distance_m: f32,  // e.g., 5.0
    pub aperture_blades:  u32,  // e.g., 6
    pub samples_per_pix:  u32,  // 1 = off; >= 2 enables RT bokeh
}

impl Dof {
    pub const OFF: Self = Dof {
        focus_distance_m: 5.0,
        aperture_blades: 6,
        samples_per_pix: 1,
    };

    pub fn aperture_radius_m(&self, lens: &Lens, exposure: &Exposure) -> f32 {
        // Physical aperture diameter = focal_length / N; radius in meters.
        (lens.focal_length_mm * 0.001) / (2.0 * exposure.aperture_fstop)
    }
}

// ── Camera modes ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CameraMode {
    Fly,
    Orbit { target: [f32; 3], distance: f32 },
}

// ── Projection override ────────────────────────────────────────────────────

/// When set on a `Camera`, overrides `fov`/`near`/`far`. `None` preserves
/// the existing perspective path — all legacy call-sites are unaffected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProjectionMode {
    Perspective  { fov: f32,  near: f32, far: f32 },
    Orthographic { size: f32, near: f32, far: f32 },
}

// ── Camera ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Camera {
    pub id:         CameraId,
    pub name:       Arc<str>,
    pub mode:       CameraMode,
    pub position:   [f32; 3],
    pub yaw:        f32,
    pub pitch:      f32,
    pub roll:       f32,
    pub fov:        f32,
    pub near:       f32,
    pub far:        f32,
    /// When `Some`, overrides `fov`/`near`/`far`. `None` = backwards-compat perspective.
    pub projection: Option<ProjectionMode>,
    pub lens:       Lens,      // NEW
    pub exposure:   Exposure,  // NEW
    pub dof:        Dof,       // NEW
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
            roll: 0.0,
            fov: 45.0_f32.to_radians(),
            near: 0.1,
            far: 100.0,
            projection: None,
            lens: Lens::FULL_FRAME,
            exposure: Exposure {
                aperture_fstop:  5.6,
                shutter_seconds: 1.0 / 60.0,
                iso:             100.0,
                ev_compensation: 0.0,
            },
            dof: Dof::OFF,
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
            roll: 0.0,
            fov: 45.0_f32.to_radians(),
            near: 0.1,
            far: 100.0,
            projection: None,
            lens: Lens::FULL_FRAME,
            exposure: Exposure {
                aperture_fstop:  5.6,
                shutter_seconds: 1.0 / 60.0,
                iso:             100.0,
                ev_compensation: 0.0,
            },
            dof: Dof::OFF,
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

    fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let forward    = Vec3::new(cp * cy, sp, cp * sy).normalize();
        let right_base = Vec3::new(-sy, 0.0, cy).normalize();
        let up_base    = right_base.cross(forward).normalize();
        let up         = Quat::from_axis_angle(forward, self.roll) * up_base;
        let right      = forward.cross(up).normalize();
        (forward, right, up)
    }

    #[inline]
    pub fn view_matrix(&self) -> [f32; 16] {
        let pos = Vec3::from(self.position);
        let (forward, _, up) = self.basis();
        Mat4::look_at_rh(pos, pos + forward, up).to_cols_array()
    }

    #[inline]
    pub fn projection_matrix(&self, aspect: f32) -> [f32; 16] {
        let mut proj = match self.projection {
            None => Mat4::perspective_rh(self.lens.fov_y(), aspect, self.near, self.far),
            Some(ProjectionMode::Perspective { fov, near, far }) => {
                Mat4::perspective_rh(fov, aspect, near, far)
            }
            Some(ProjectionMode::Orthographic { size, near, far }) => {
                let hw = size * aspect * 0.5;
                let hh = size * 0.5;
                Mat4::orthographic_rh(-hw, hw, -hh, hh, near, far)
            }
        };
        // Vulkan NDC: Y points down — flip the Y axis of the projection.
        proj.y_axis.y = -proj.y_axis.y;
        proj.to_cols_array()
    }

    #[inline]
    pub fn view_projection_matrix(&self, aspect: f32) -> [f32; 16] {
        let (forward, _, up) = self.basis();
        let pos  = Vec3::from(self.position);
        let view = Mat4::look_at_rh(pos, pos + forward, up);
        let mut proj = match self.projection {
            None => Mat4::perspective_rh(self.lens.fov_y(), aspect, self.near, self.far),
            Some(ProjectionMode::Perspective { fov, near, far }) =>
                Mat4::perspective_rh(fov, aspect, near, far),
            Some(ProjectionMode::Orthographic { size, near, far }) => {
                let hw = size * aspect * 0.5;
                let hh = size * 0.5;
                Mat4::orthographic_rh(-hw, hw, -hh, hh, near, far)
            }
        };
        proj.y_axis.y = -proj.y_axis.y;
        (proj * view).to_cols_array()
    }

    /// Translate position in the camera's local right/up plane (pan for all pane types).
    #[inline]
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let (_, right, up) = self.basis();
        let delta = right * dx + up * dy;
        self.position[0] += delta.x;
        self.position[1] += delta.y;
        self.position[2] += delta.z;
    }

    /// Zoom: shrinks orthographic `size` or flies perspective forward.
    #[inline]
    pub fn zoom(&mut self, delta: f32) {
        match &mut self.projection {
            Some(ProjectionMode::Orthographic { size, .. }) => {
                *size = (*size * (1.0 - delta * 0.1)).max(0.01);
            }
            _ => self.move_forward(delta * 0.5),
        }
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
    forward:           bool,
    backward:          bool,
    left:              bool,
    right:             bool,
    up:                bool,
    down:              bool,
    roll_left:         bool,
    roll_right:        bool,
    mouse_grabbed:     bool,
    middle_grabbed:    bool,
    mouse_sensitivity: f32,
    move_speed:        f32,
    roll_speed:        f32,
    pan_sensitivity:   f32,
}

impl CameraController {
    pub fn new(move_speed: f32, mouse_sensitivity: f32) -> Self {
        Self {
            forward:           false,
            backward:          false,
            left:              false,
            right:             false,
            up:                false,
            down:              false,
            roll_left:         false,
            roll_right:        false,
            mouse_grabbed:     false,
            middle_grabbed:    false,
            move_speed,
            mouse_sensitivity,
            roll_speed:        1.0,
            pan_sensitivity:   0.01,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode, state: ElementState) {
        let pressed = state == ElementState::Pressed;
        match key {
            KeyCode::KeyW => self.forward    = pressed,
            KeyCode::KeyS => self.backward   = pressed,
            KeyCode::KeyA => self.left       = pressed,
            KeyCode::KeyD => self.right      = pressed,
            KeyCode::KeyQ => self.up         = pressed,
            KeyCode::KeyE => self.down       = pressed,
            KeyCode::KeyZ => self.roll_left  = pressed,
            KeyCode::KeyC => self.roll_right = pressed,
            _ => {}
        }
    }

    pub fn handle_mouse(&mut self, dx: f32, dy: f32) -> (f32, f32) {
        if !self.mouse_grabbed {
            return (0.0, 0.0);
        }
        let yaw   = -dx * self.mouse_sensitivity;
        let pitch = -dy * self.mouse_sensitivity;
        (yaw, pitch)
    }

    /// Call when the middle mouse button state changes. Returns new grabbed state.
    pub fn handle_middle_button(&mut self, state: ElementState) -> bool {
        self.middle_grabbed = state == ElementState::Pressed;
        self.middle_grabbed
    }

    /// Pan the camera in its local right/up plane when middle-mouse is grabbed.
    #[inline]
    pub fn handle_pan(&self, camera: &mut Camera, dx: f32, dy: f32) {
        if self.middle_grabbed {
            camera.pan(dx * self.pan_sensitivity, -dy * self.pan_sensitivity);
        }
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

    pub fn is_middle_grabbed(&self) -> bool {
        self.middle_grabbed
    }

    pub fn pan_sensitivity(&self) -> f32 {
        self.pan_sensitivity
    }

    pub fn update(&self, camera: &mut Camera, dt: f32) {
        let speed = self.move_speed * dt;
        if self.forward    { camera.move_forward(speed);  }
        if self.backward   { camera.move_forward(-speed); }
        if self.right      { camera.move_right(speed);    }
        if self.left       { camera.move_right(-speed);   }
        if self.up         { camera.move_up(speed);       }
        if self.down       { camera.move_up(-speed);      }
        if self.roll_left  { camera.roll += self.roll_speed * dt; }
        if self.roll_right { camera.roll -= self.roll_speed * dt; }
    }
}

// ── Push Constants ─────────────────────────────────────────────────────────

/// Scene (raster) push constants. Packed for 128 B (Vulkan typical limit).
#[repr(C)]
pub struct ScenePush {
    pub view_proj:      [f32; 16],
    pub cam_pos:        [f32; 3],
    pub _pad0:          f32,
    pub cam_forward:    [f32; 3],
    pub _pad1:          f32,
    pub exposure_scale: f32,
    pub _pad2:          [f32; 3],
}

/// Ray-tracing push constants. Exactly 128 B (one 4x32-bit register block).
#[repr(C)]
pub struct RayCameraPush {
    pub inv_vp:         [f32; 16],
    pub cam_pos:        [f32; 3],
    pub is_ortho:       u32,
    pub cam_forward:    [f32; 3],
    pub frame_count:    u32,
    pub aperture_radius: f32,
    pub focus_distance: f32,
    pub samples_per_pix: u32,
    pub aperture_blades: u32,
    pub pane_offset_px: [u32; 2],
    pub pane_size_px:   [u32; 2],
    pub exposure_scale: f32,
    pub _pad:           [f32; 3],
}
