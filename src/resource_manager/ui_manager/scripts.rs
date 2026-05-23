//! Maps UI widget actions to engine `Effect` queue entries.
//!
//! Each interactive widget (Button, Slider, Dropdown, Checkbox) may carry
//! an `Option<UiActionId>` — when fired (button click, slider drag end,
//! etc.) the action id is resolved here into a concrete `Effect::UiAction`
//! and pushed onto the per-tick effect arena.

use crate::resource_manager::event_manager::{Effect, EffectArena};

/// Stable identifier for a UI-bound action. Resolved by the consumer
/// (e.g. EditorApp) into a concrete handler. The identifier is opaque
/// to UiManager — it only routes the dispatch.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct UiActionId(pub u32);

/// Push a `UiAction` effect onto the per-tick effect arena. Returns the
/// effect index for diagnostics.
pub fn dispatch_ui_action(arena: &mut EffectArena, action: UiActionId, payload: f32) -> usize {
    let idx = arena.len();
    arena.push(Effect::UiAction { action: action.0, payload });
    idx
}
