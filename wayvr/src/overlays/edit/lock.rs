use std::rc::Rc;

use anyhow::Context;
use glam::FloatExt;
use wgui::{
    animation::{Animation, AnimationEasing},
    components::button::ComponentButton,
    event::CallbackDataCommon,
    layout::WidgetID,
    parser::Fetchable,
    widget::rectangle::WidgetRectangle,
};

use crate::{backend::task::ModifyOverlayTask, overlays::edit::EditModeWrapPanel};

#[derive(Default)]
pub(super) struct InteractLockHandler {
    id: WidgetID,
    color: wgui::drawing::Color,
    interactable: bool,
    button: Option<Rc<ComponentButton>>,
}

impl InteractLockHandler {
    pub fn new(panel: &mut EditModeWrapPanel) -> anyhow::Result<Self> {
        let id = panel.parser_state.get_widget_id("shadow")?;
        let shadow_rect = panel
            .layout
            .state
            .widgets
            .get_as::<WidgetRectangle>(id)
            .context("Element with id=\"shadow\" must be a <rectangle>")?;

        let button = panel.parser_state.fetch_component_as("top_lock")?;

        Ok(Self {
            id,
            color: shadow_rect.params.color,
            interactable: true,
            button: Some(button),
        })
    }

    pub fn reset(&mut self, common: &mut CallbackDataCommon, interactable: bool) {
        self.interactable = interactable;
        let mut rect = common
            .state
            .widgets
            .get_as::<WidgetRectangle>(self.id)
            .unwrap(); // can only fail if set_up_rect has issues

        if let Some(button) = self.button.as_ref() {
            button.set_sticky_state(common, !interactable);
        }

        if interactable {
            set_anim_color(&mut rect, 0.0, self.color, common.state.theme.danger_color);
        } else {
            set_anim_color(&mut rect, 0.2, self.color, common.state.theme.danger_color);
        }
    }

    pub fn toggle(
        &mut self,
        common: &mut CallbackDataCommon,
        anim_mult: f32,
    ) -> Box<ModifyOverlayTask> {
        let rect_color = self.color;

        self.interactable = !self.interactable;
        if let Some(button) = self.button.as_ref() {
            button.set_sticky_state(common, !self.interactable);
        }

        let anim = if self.interactable {
            Animation::new(
                self.id,
                (10. * anim_mult) as _,
                AnimationEasing::OutQuad,
                Box::new(move |common, data| {
                    let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
                    set_anim_color(
                        rect,
                        0.2 - (data.pos * 0.2),
                        rect_color,
                        common.state.theme.danger_color,
                    );
                    common.alterables.mark_redraw();
                }),
            )
        } else {
            Animation::new(
                self.id,
                (10. * anim_mult) as _,
                AnimationEasing::OutBack,
                Box::new(move |common, data| {
                    let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
                    set_anim_color(
                        rect,
                        data.pos * 0.2,
                        rect_color,
                        common.state.theme.danger_color,
                    );
                    common.alterables.mark_redraw();
                }),
            )
        };

        common.alterables.animate(anim);

        let interactable = self.interactable;
        Box::new(move |_app, owc| {
            let state = owc.active_state.as_mut().unwrap(); //want panic
            state.interactable = interactable;
        })
    }
}

fn set_anim_color(
    rect: &mut WidgetRectangle,
    pos: f32,
    rect_color: wgui::drawing::Color,
    target_color: wgui::drawing::Color,
) {
    // rect to target_color
    rect.params.color.r = rect_color.r.lerp(target_color.r, pos);
    rect.params.color.g = rect_color.g.lerp(target_color.g, pos);
    rect.params.color.b = rect_color.b.lerp(target_color.b, pos);
}
