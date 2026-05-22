mod sealed {
    pub trait Sealed {}
}

use std::sync::Arc;
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
    pub scale:    (f32, f32, f32),
    pub _transform: bool,
}

#[derive(Debug)]
pub struct AudioComponent {
    pub volume:   f32,
    pub pitch:    f32,
    pub _loop:    bool,
    pub _playing: bool,
}

#[derive(Debug)]
pub struct PhysicsComponent {
    pub mass:         f32,
    pub velocity:     (f32, f32, f32),
    pub acceleration: (f32, f32, f32),
}

#[derive(Debug)]
pub struct CollisionComponent {
    pub shape:     CollisionShape,
    pub position:  (f32, f32, f32),
    pub rotation:  (f32, f32, f32),
    pub scale:     (f32, f32, f32),
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
    pub color:     [f32; 3],
    pub intensity: f32,
    /// Sphere radius around the actor's world position for point/spot lights.
    pub range:     f32,
    pub kind:      LightKind,
}

#[derive(Debug, Clone)]
pub enum LightKind {
    Point,
    Spot        { half_angle: f32, direction: [f32; 3] },
    Directional { direction: [f32; 3] },
}

/// Per-actor animation + pose state. Owned by UtilityComponent.render on
/// actors that carry a mesh and need independent animation state.
#[derive(Debug)]
pub struct RenderState {
    pub pose:       crate::resource_manager::asset_manager::forge_gltf::Pose,
    pub anim_index: Option<usize>,
    pub anim_time:  f32,
}

#[derive(Debug)]
pub struct UtilityComponent {
    pub name:        Arc<str>,
    pub description: Arc<str>,
    pub camera:      Option<CameraData>,
    pub light:       Option<LightData>,
    pub render:      Option<RenderState>,
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
}
