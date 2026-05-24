use thin_vec::ThinVec;

use crate::render::ui_core::layout::{Alignment, ColumnLayout, LayoutDispatch};
use crate::render::ui_core::signal::Signal;
use crate::render::ui_core::widget::{
    ButtonState, CheckboxState, DirtyFlags, LabelState, PanelState, SliderState, Widget, WidgetKind,
};
use crate::render::ui_core::{Constraint, Rect, UiEvent, UiManager, WidgetId, WidgetIdPath};

/// Placeholder inserted before the arena hands back the real generational id.
/// Immediately overwritten after `Arena::insert` returns.
fn sentinel_id() -> WidgetId {
    WidgetId {
        idx: u32::MAX,
        generation: unsafe { std::num::NonZeroU32::new_unchecked(1) },
        _tag: std::marker::PhantomData,
    }
}

pub struct UiBuilder<'a> {
    manager: &'a mut UiManager,
    current_parent: Option<WidgetId>,
    pub cursor: [f32; 2],
    gap: f32,
    id_stack: ThinVec<&'static str>,
}

impl<'a> UiBuilder<'a> {
    pub fn new(manager: &'a mut UiManager) -> Self {
        Self {
            manager,
            current_parent: None,
            cursor: [0., 0.],
            gap: 4.,
            id_stack: ThinVec::new(),
        }
    }

    /// Return an existing widget id for `name` in the current path scope, or
    /// allocate a new one via the arena and register the path→id mapping.
    fn resolve_or_create(&mut self, name: &'static str, kind: WidgetKind, height: f32) -> WidgetId {
        self.id_stack.push(name);
        let path = WidgetIdPath(self.id_stack.clone()).to_string();
        self.id_stack.pop();

        if let Some(id) = self.manager.get_widget_by_path(&path) {
            return id;
        }

        let x = self.cursor[0];
        let y = self.cursor[1];
        let vw = self.manager.viewport_width();

        let id = self.manager.widgets.insert(Widget {
            id: sentinel_id(), // patched immediately below
            kind,
            parent: self.current_parent,
            children: ThinVec::new(),
            dirty: DirtyFlags::LAYOUT as u8 | DirtyFlags::CONTENT as u8,
            rect: Rect { x, y, w: vw, h: height },
            constraint: Constraint {
                min_width: 0.0,
                max_width: vw,
                min_height: height,
                max_height: height,
            },
            layout_solver: LayoutDispatch::Null,
            event_handlers: ThinVec::new(),
            user_data: None,
        });

        // Patch the self-referential id field now that the arena has assigned it.
        if let Some(w) = self.manager.widgets.get_mut(id) {
            w.id = id;
        }

        self.manager.register_widget_path(path, id);

        if let Some(parent_id) = self.current_parent {
            if let Some(parent) = self.manager.widgets.get_mut(parent_id) {
                parent.children.push(id);
            }
        }

        id
    }

    pub fn label(&mut self, text: &str) {
        let state = LabelState {
            text: Signal::new(text.to_owned()),
            color: Signal::new([1.0, 1.0, 1.0, 1.0]),
            font_size: 14,
        };
        let id = self.resolve_or_create("label", WidgetKind::Label(state), 20.0);
        if let Some(w) = self.manager.widgets.get_mut(id) {
            if let WidgetKind::Label(ref mut s) = w.kind {
                s.text.set(text.to_owned());
            }
        }
        self.cursor[1] += 20.0 + self.gap;
    }

    pub fn button(&mut self, text: &str) -> bool {
        let state = ButtonState {
            label: Signal::new(text.to_owned()),
            enabled: Signal::new(true),
            clicked: false,
        };
        let id = self.resolve_or_create("button", WidgetKind::Button(state), 30.0);

        let clicked = self
            .manager
            .widgets
            .get(id)
            .map(|w| matches!(&w.kind, WidgetKind::Button(s) if s.clicked))
            .unwrap_or(false);

        if clicked {
            if let Some(w) = self.manager.widgets.get_mut(id) {
                if let WidgetKind::Button(ref mut s) = w.kind {
                    s.clicked = false;
                }
            }
            self.manager.event_bus.emit(UiEvent::Click(id));
        }

        self.cursor[1] += 30.0 + self.gap;
        clicked
    }

    pub fn slider(&mut self, _label: &str, min: f32, max: f32, value: &mut f32) {
        let state = crate::render::ui_core::widget::SliderState {
            value: Signal::new(*value),
            min,
            max,
            step: 0.01,
            dragging: false,
        };
        let id = self.resolve_or_create("slider", WidgetKind::Slider(state), 25.0);

        if let Some(w) = self.manager.widgets.get(id) {
            if let WidgetKind::Slider(ref s) = w.kind {
                *value = s.value.get();
            }
        }

        self.cursor[1] += 25.0 + self.gap;
    }

    pub fn checkbox(&mut self, _label: &str, value: &mut bool) {
        let state = CheckboxState {
            checked: Signal::new(*value),
            label: Signal::new(String::new()),
        };
        let id = self.resolve_or_create("checkbox", WidgetKind::Checkbox(state), 20.0);

        if let Some(w) = self.manager.widgets.get(id) {
            if let WidgetKind::Checkbox(ref s) = w.kind {
                *value = s.checked.get();
            }
        }

        self.cursor[1] += 20.0 + self.gap;
    }

    pub fn begin_column(&mut self, name: &'static str) {
        let state = PanelState {
            title: Signal::new(name.to_owned()),
            closable: false,
            close_requested: false,
        };
        let id = self.resolve_or_create(name, WidgetKind::Panel(state), 0.0);
        if let Some(w) = self.manager.widgets.get_mut(id) {
            w.layout_solver = LayoutDispatch::Column(ColumnLayout {
                gap: self.gap,
                cross_alignment: Alignment::Start,
            });
        }
        self.id_stack.push(name);
        self.current_parent = Some(id);
    }

    pub fn end_column(&mut self) {
        self.id_stack.pop();
        self.current_parent = self
            .current_parent
            .and_then(|id| self.manager.widgets.get(id))
            .and_then(|w| w.parent);
    }
}
