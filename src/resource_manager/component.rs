mod sealed {
    pub trait Sealed {}
}

use std::sync::Arc;
pub trait ComponentData: sealed::Sealed {
    const TYPE: ComponentType;
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

#[derive(Debug)]
pub struct UtilityComponent {
    pub name:        Arc<str>,
    pub description: Arc<str>,
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
