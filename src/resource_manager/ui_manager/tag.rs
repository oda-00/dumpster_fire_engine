use crate::resource_manager::manager::Handle;

/// Marker tag for panel arena entries.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub struct PanelTag;
pub type PanelHandle = Handle<PanelTag>;

/// Marker tag for widget arena entries.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub struct WidgetTag;
pub type WidgetHandle = Handle<WidgetTag>;

/// Marker tag for font/icon atlas entries.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub struct AtlasTag;
pub type AtlasHandle = Handle<AtlasTag>;
