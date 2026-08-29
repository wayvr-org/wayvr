use std::rc::Rc;

use anyhow::Context;
use wgui::{
    animation::{Animation, AnimationDuration, AnimationEasing},
    color::{WguiColor, WguiColorName},
    components::button::ComponentButton,
    event::CallbackDataCommon,
    layout::WidgetID,
    palette::WguiColorPalette,
    parser::Fetchable,
    widget::rectangle::WidgetRectangle,
};

use crate::{backend::task::ModifyOverlayTask, overlays::edit::EditModeWrapPanel};

#[derive(Default)]
pub(super) struct InteractLockHandler {
    id: WidgetID,
    color: WguiColor,
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

        let globals = common.globals();
        let palette = &globals.palette;

        if interactable {
            set_anim_color(
                palette,
                &mut rect,
                0.0,
                self.color,
                WguiColorName::Danger.into(),
            );
        } else {
            set_anim_color(
                palette,
                &mut rect,
                0.2,
                self.color,
                WguiColorName::Danger.into(),
            );
        }
    }

    pub fn toggle(&mut self, common: &mut CallbackDataCommon) -> Box<ModifyOverlayTask> {
        let rect_color = self.color;

        self.interactable = !self.interactable;
        if let Some(button) = self.button.as_ref() {
            button.set_sticky_state(common, !self.interactable);
        }

        let anim = if self.interactable {
            Animation::new(
                self.id,
                AnimationDuration::Seconds(0.1666),
                AnimationEasing::OutQuad,
                Box::new(move |common, data| {
                    let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
                    set_anim_color(
                        &common.globals().palette,
                        rect,
                        0.2 - (data.pos * 0.2),
                        rect_color,
                        WguiColorName::Danger.into(),
                    );
                    common.alterables.mark_redraw();
                }),
            )
        } else {
            Animation::new(
                self.id,
                AnimationDuration::Seconds(0.1666),
                AnimationEasing::OutBack,
                Box::new(move |common, data| {
                    let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
                    set_anim_color(
                        &common.globals().palette,
                        rect,
                        data.pos * 0.2,
                        rect_color,
                        WguiColorName::Danger.into(),
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
    palette: &WguiColorPalette,
    rect: &mut WidgetRectangle,
    pos: f32,
    rect_color: WguiColor,
    target_color: WguiColor,
) {
    // rect to target_color
    rect.params.color = rect_color.lerp(palette, &target_color, pos);
}
