use thin_vec::ThinVec;
use crate::render::ui_core::{UiManager, WidgetId, WidgetIdPath, UiEvent};

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

    fn widget_id(&mut self, local_name: &'static str) -> WidgetId {
        self.id_stack.push(local_name);
        let path = WidgetIdPath(self.id_stack.clone()).to_string();

        if let Some(id) = self.manager.get_widget_by_path(&path) {
            self.id_stack.pop();
            return id;
        }

        self.id_stack.pop();

        use std::marker::PhantomData;
        WidgetId {
            idx: 0,
            generation: unsafe { std::num::NonZeroU32::new_unchecked(1) },
            _tag: PhantomData,
        }
    }

    pub fn label(&mut self, text: &str) {
        let _id = self.widget_id("label");
        self.cursor[1] += 20.0 + self.gap;
    }

    pub fn button(&mut self, text: &str) -> bool {
        let _id = self.widget_id("button");
        self.cursor[1] += 30.0 + self.gap;
        false
    }

    pub fn slider(&mut self, label: &str, min: f32, max: f32, value: &mut f32) {
        let _id = self.widget_id("slider");
        self.cursor[1] += 25.0 + self.gap;
    }

    pub fn checkbox(&mut self, label: &str, value: &mut bool) {
        let _id = self.widget_id("checkbox");
        self.cursor[1] += 20.0 + self.gap;
    }

    fn place_widget(&mut self, height: f32) {
        self.cursor[1] += height + self.gap;
    }
}
