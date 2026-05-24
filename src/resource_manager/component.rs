mod sealed {
    pub trait Sealed {}
}

use std::sync::Arc;
use thin_vec::ThinVec;

use crate::resource_manager::manager::ActorHandle;

pub trait ComponentData: sealed::Sealed {
    const TYPE: ComponentType;
}

// ── GltfHandle — mesh asset reference ────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct GltfTag;
pub type GltfHandle = crate::resource_manager::manager::Handle<GltfTag>;

/// A lightweight reference from an actor sub-entity to a loaded glTF asset.
/// Copy so it can live inside ThinVec and arena operations without cloning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshRef {
    pub asset: GltfHandle,
}

/// Generates ComponentType enum, Component enum, From impls, and ComponentData impls
/// from a list of `Variant: DataType` pairs. Single source of truth for component count.
macro_rules! declare_components {
    ($($variant:ident : $data:ty),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum ComponentType {
            $($variant),+
        }

        impl ComponentType {
            pub const COUNT: usize = [$( stringify!($variant) ),+].len();

            #[inline]
            pub const fn index(self) -> usize { self as usize }

            pub const ALL: [ComponentType; Self::COUNT] = [$(ComponentType::$variant),+];
        }

        #[derive(Debug)]
        pub enum Component {
            $($variant($data)),+
        }

        impl Component {
            pub fn component_type(&self) -> ComponentType {
                match self {
                    $(Component::$variant(_) => ComponentType::$variant,)+
                }
            }

            #[inline]
            pub fn index(&self) -> usize { self.component_type().index() }
        }

        $(
            impl From<$data> for Component {
                fn from(d: $data) -> Self { Component::$variant(d) }
            }

            impl TryFrom<Component> for $data {
                type Error = ();
                fn try_from(c: Component) -> Result<Self, ()> {
                    match c {
                        Component::$variant(d) => Ok(d),
                        _ => Err(()),
                    }
                }
            }

            impl sealed::Sealed for $data {}

            impl ComponentData for $data {
                const TYPE: ComponentType = ComponentType::$variant;
            }
        )+
    };
}

// ── Component data structs ──────────────────────────────────────────────────

#[derive(Debug)]
pub struct TransformComponent {
    pub position: (f32, f32, f32),
    pub rotation: (f32, f32, f32),
    pub scale: (f32, f32, f32),
    pub _transform: bool,
}

#[derive(Debug)]
pub struct AudioComponent {
    pub volume: f32,
    pub pitch: f32,
    pub _loop: bool,
    pub _playing: bool,
}

#[derive(Debug)]
pub struct PhysicsComponent {
    pub mass: f32,
    pub velocity: (f32, f32, f32),
    pub acceleration: (f32, f32, f32),
}

#[derive(Debug)]
pub struct CollisionComponent {
    pub shape: CollisionShape,
    pub position: (f32, f32, f32),
    pub rotation: (f32, f32, f32),
    pub scale: (f32, f32, f32),
    pub collision: bool,
}

/// Camera sub-state owned by a Utility actor. Reuses the engine Camera struct.
#[derive(Debug, Clone)]
pub struct CameraData {
    pub camera: crate::render::camera::Camera,
}

/// Light sub-state owned by a Utility actor.
#[derive(Debug, Clone)]
pub struct LightData {
    pub color: [f32; 3],
    pub intensity: f32,
    /// Sphere radius around the actor's world position for point/spot lights.
    pub range: f32,
    pub kind: LightKind,
}

// ── Light substrate handles ─────────────────────────────────────────────────

/// Index into the engine-wide bindless IES profile array (set 0 binding 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IesHandle(pub u32);

/// Index into the engine-wide bindless HDRI cubemap array (set 0 binding 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HdriHandle(pub u32);

/// Index into the engine-wide bindless 3D density-texture array (set 0 binding 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DensityTexHandle(pub u32);

/// Analytic-sky parametric model. Encoded as `model_id_f` in `LightGpu.data[1].w`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SkyModel {
    HosekWilkie = 0,
    Preetham = 1,
    AtmosphericScattering = 2,
}

/// Physically-complete light taxonomy. Twenty variants covering every
/// distinct combination of geometry / medium / data-source we want to ship.
///
/// Indexing convention (kept stable; matches the GPU `kind` tag in `LightGpu`):
///
///   0 Point        5 Disk         10 Volumetric    15 Ies
///   1 Spot         6 Rectangle    11 VolumeBox     16 Mesh
///   2 Directional  7 Polygon      12 VolumeCone    17 Environment
///   3 Sun          8 Linear       13 VolumeCylinder 18 AnalyticSky
///   4 Sphere       9 Tube         14 VolumeMesh    19 Ambient
#[derive(Debug, Clone)]
pub enum LightKind {
    // ── Punctual + sun-disc ────────────────────────────────────────────
    Point,
    Spot {
        cone_inner: f32,
        cone_outer: f32,
        direction: [f32; 3],
    },
    Directional {
        direction: [f32; 3],
    },
    Sun {
        direction: [f32; 3],
        angular_radius: f32,
    },

    // ── Area lights ────────────────────────────────────────────────────
    Sphere {
        radius: f32,
    },
    Disk {
        normal: [f32; 3],
        radius: f32,
        two_sided: bool,
    },
    Rectangle {
        normal: [f32; 3],
        tangent: [f32; 3],
        size: [f32; 2],
        two_sided: bool,
    },
    Polygon {
        normal: [f32; 3],
        tangent: [f32; 3],
        vertices: ThinVec<[f32; 2]>,
        two_sided: bool,
    },
    Linear {
        point_b: [f32; 3],
        radius: f32,
    },
    Tube {
        point_b: [f32; 3],
        radius: f32,
        capped: bool,
    },

    // ── Participating media ────────────────────────────────────────────
    Volumetric {
        radius: f32,
        extinction: f32,
        anisotropy_g: f32,
    },
    VolumeBox {
        half_extents: [f32; 3],
        extinction: f32,
        anisotropy_g: f32,
    },
    VolumeCone {
        direction: [f32; 3],
        half_angle: f32,
        height: f32,
        extinction: f32,
        anisotropy_g: f32,
    },
    VolumeCylinder {
        direction: [f32; 3],
        height: f32,
        radius: f32,
        extinction: f32,
        anisotropy_g: f32,
    },
    VolumeMesh {
        mesh_actor: ActorHandle,
        density_tex: Option<DensityTexHandle>,
        extinction: f32,
        anisotropy_g: f32,
    },

    // ── Data-driven photometric ────────────────────────────────────────
    Ies {
        direction: [f32; 3],
        profile: IesHandle,
    },

    // ── Global / special ───────────────────────────────────────────────
    Mesh {
        mesh_actor: ActorHandle,
    },
    Environment {
        hdri: HdriHandle,
        rotation_rad: f32,
        intensity_scale: f32,
    },
    AnalyticSky {
        sun_direction: [f32; 3],
        turbidity: f32,
        ground_albedo: [f32; 3],
        model: SkyModel,
    },
    Ambient,
}

impl LightKind {
    /// Stable GPU tag matching the chit / frag shader's `kind` dispatch.
    pub const fn tag(&self) -> u32 {
        match self {
            LightKind::Point => 0,
            LightKind::Spot { .. } => 1,
            LightKind::Directional { .. } => 2,
            LightKind::Sun { .. } => 3,
            LightKind::Sphere { .. } => 4,
            LightKind::Disk { .. } => 5,
            LightKind::Rectangle { .. } => 6,
            LightKind::Polygon { .. } => 7,
            LightKind::Linear { .. } => 8,
            LightKind::Tube { .. } => 9,
            LightKind::Volumetric { .. } => 10,
            LightKind::VolumeBox { .. } => 11,
            LightKind::VolumeCone { .. } => 12,
            LightKind::VolumeCylinder { .. } => 13,
            LightKind::VolumeMesh { .. } => 14,
            LightKind::Ies { .. } => 15,
            LightKind::Mesh { .. } => 16,
            LightKind::Environment { .. } => 17,
            LightKind::AnalyticSky { .. } => 18,
            LightKind::Ambient => 19,
        }
    }

    /// True when the kind has a meaningful world-space position (point-like
    /// or area). False for Directional/Sun/Environment/AnalyticSky/Ambient,
    /// which are inherently positionless.
    pub const fn is_positional(&self) -> bool {
        !matches!(
            self,
            LightKind::Directional { .. }
                | LightKind::Sun { .. }
                | LightKind::Environment { .. }
                | LightKind::AnalyticSky { .. }
                | LightKind::Ambient
        )
    }
}

/// Per-actor animation + pose state. Owned by UtilityComponent.render on
/// actors that carry a mesh and need independent animation state.
#[derive(Debug)]
pub struct RenderState {
    pub pose: crate::resource_manager::asset_manager::forge_gltf::Pose,
    pub anim_index: Option<usize>,
    pub anim_time: f32,
}

#[derive(Debug)]
pub struct UtilityComponent {
    pub name: Arc<str>,
    pub description: Arc<str>,
    pub camera: Option<CameraData>,
    pub light: Option<LightData>,
    pub render: Option<RenderState>,
}

/// Billboard-anchored UI panel — for in-world labels (light gizmo names,
/// debug overlays). The `panel` handle points into UiManager.panels.
#[derive(Debug)]
pub struct UiComponent {
    pub panel: crate::resource_manager::ui_manager::PanelHandle,
    pub world_offset: [f32; 3],
    pub size_px: [u32; 2],
}

#[derive(Debug)]
pub enum CollisionShape {
    Box,
    Sphere,
    Capsule,
    Mesh,
}

// ── Macro invocation ────────────────────────────────────────────────────────

declare_components! {
    Transform: TransformComponent,
    Audio:     AudioComponent,
    Physics:   PhysicsComponent,
    Collision: CollisionComponent,
    Utility:   UtilityComponent,
    Ui:        UiComponent,
}
