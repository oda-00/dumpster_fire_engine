use super::atlas::Atlas;
use super::draw::DrawList;
use super::input::UiInputState;
use super::layout::Rect;
use super::panel::{Panel, PanelHandle, PanelTag};
use super::widget::{Widget, WidgetTag};
use crate::resource_manager::manager::Arena;
use thin_vec::ThinVec;

pub struct UiManager {
    pub panels: Arena<PanelTag, Panel>,
    pub widgets: Arena<WidgetTag, Widget>,
    pub root: ThinVec<PanelHandle>,
    pub draw_list: DrawList,
    pub atlas: Atlas,
    pub input: UiInputState,
    pub dirty: bool,
    /// True when the most recent pointer event landed inside any visible
    /// panel rect — toggled by the consumer when it routes input. Cleared
    /// each tick.
    pub input_consumed: bool,
}

impl UiManager {
    pub fn new() -> Self {
        Self {
            panels: Arena::new(),
            widgets: Arena::new(),
            root: ThinVec::new(),
            draw_list: DrawList::new(),
            atlas: Atlas::build(),
            input: UiInputState::default(),
            dirty: false,
            input_consumed: false,
        }
    }

    /// Cascade-tick entry point. Called by `World::tick` after world logic.
    /// Order: clear draw list → mark which panels swallowed input →
    /// recurse `Panel::tick → Widget::tick`. Input state is *not* cleared
    /// here; the consumer (editor / game) advances it via `input.end_frame()`
    /// after each frame's events drain.
    pub fn tick(&mut self, input: &UiInputState, dt: f32) {
        self.input = input.clone();
        self.draw_list.clear();
        self.input_consumed = false;

        // Hit-test against panel rects to decide if UI ate the pointer.
        if input.left_just_pressed {
            for &ph in &self.root {
                if let Some(p) = self.panels.get(ph)
                    && p.visible
                    && p.rect.contains(input.cursor[0], input.cursor[1])
                {
                    self.input_consumed = true;
                    break;
                }
            }
        }

        // Cascade: Panel → Widget. We split-borrow widgets out so each
        // Panel.tick can mutate widget state while the Panel itself stays
        // immutably referenced through `self.panels`.
        let panels_snapshot: ThinVec<PanelHandle> = self.panels.entries().map(|(h, _)| h).collect();
        for ph in panels_snapshot {
            if let Some(panel) = self.panels.get_mut(ph) {
                panel.tick(&mut self.widgets, input, dt);
            }
        }
    }

    /// True when the UI consumed the most recent pointer event (clears each tick).
    pub fn consumed_input(&self) -> bool {
        self.input_consumed
    }

    /// True when the selection changed this tick (triggers inspector rebuild).
    pub fn selection_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark the inspector as dirty so the next tick rebuilds its panel.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Build the inspector panel via the immediate-mode API.
    pub fn frame_inspector<F: FnOnce(&mut super::immediate::Ui<'_>)>(&mut self, f: F) {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 320.0,
            h: 600.0,
        };
        let mut ui = super::immediate::Ui::new(&mut self.draw_list, rect);
        f(&mut ui);
    }

    /// Spawn a retained panel and register it as a root.
    pub fn spawn_panel(&mut self, panel: Panel) -> PanelHandle {
        let h = self.panels.insert(panel);
        self.root.push(h);
        h
    }
}

impl Default for UiManager {
    fn default() -> Self {
        Self::new()
    }
}
