
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ComponentType {
    Transform = 0,
    Audio     = 1,
    Physics   = 2,
    Collision = 3,
    Utility   = 4,
}

impl ComponentType {
    pub const COUNT: usize = 5;

    #[inline]
    pub const fn index(self) -> usize { self as usize }
}

#[derive(Debug)]
pub enum Component{
    Transform(TransformComponent),
    Audio(AudioComponent),
    Physics(PhysicsComponent),
    Collision(CollisionComponent),
    Utility(UtilityComponent),
}

impl Component {
    pub fn component_type(&self) -> ComponentType {
        match self {
            Component::Transform(_) => ComponentType::Transform,
            Component::Audio(_) => ComponentType::Audio,
            Component::Physics(_) => ComponentType::Physics,
            Component::Collision(_) => ComponentType::Collision,
            Component::Utility(_) => ComponentType::Utility,
        }
    }
}

#[derive(Debug)]
pub struct TransformComponent{
    pub position: (f32, f32, f32),
    pub rotation: (f32, f32, f32),
    pub scale: (f32, f32, f32),
    pub _transform: bool,
}

#[derive(Debug)]
pub struct AudioComponent{
    pub volume: f32,
    pub pitch: f32,
    pub _loop: bool,
    pub _playing: bool,
}

#[derive(Debug)]
pub struct PhysicsComponent{
    pub mass: f32,
    pub velocity: (f32, f32, f32),
    pub acceleration: (f32, f32, f32),
}

#[derive(Debug)]
pub struct CollisionComponent{
    pub shape: CollisionShape,
    pub position: (f32, f32, f32),
    pub rotation: (f32, f32, f32),
    pub scale: (f32, f32, f32),
    pub collision: bool,
}

#[derive(Debug)]
pub struct UtilityComponent{
    pub name: String,
    pub description: String,
}

#[derive(Debug)]
pub enum CollisionShape{
    Box,
    Sphere,
    Capsule,
    Mesh,
}
