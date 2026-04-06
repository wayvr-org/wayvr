use std::{collections::HashMap, rc::Rc};

use wgui::{
    components::button::ComponentButton, event::CallbackDataCommon, layout::WidgetID,
    parser::Fetchable, renderer_vk::text::custom_glyph::CustomGlyphData,
    widget::sprite::WidgetSprite,
};

use crate::{backend::task::ModifyOverlayTask, overlays::edit::EditModeWrapPanel};

pub trait SpriteTabKey {
    fn to_tab_key(&self) -> &'static str;
    fn from_tab_key(key: &str) -> Self;
}

struct SpriteTabButtonState<S> {
    sprite: CustomGlyphData,
    component: Rc<ComponentButton>,
    state: S,
}

pub type SpriteTabHighlightChanged<S> = dyn Fn(&mut CallbackDataCommon, &S);
pub type SpriteTabButtonClicked<S> = dyn Fn(&mut CallbackDataCommon, &S) -> Box<ModifyOverlayTask>;

#[derive(Default)]
pub(super) struct SpriteTabHandler<S> {
    top_sprite_id: WidgetID,
    buttons: HashMap<&'static str, Rc<SpriteTabButtonState<S>>>,
    active_button: Option<Rc<SpriteTabButtonState<S>>>,
    on_highlight_changed: Option<Box<SpriteTabHighlightChanged<S>>>,
    on_button_clicked: Option<Box<SpriteTabButtonClicked<S>>>,
}

impl<S> SpriteTabHandler<S>
where
    S: SpriteTabKey,
{
    pub fn new(
        panel: &mut EditModeWrapPanel,
        prefix: &str,
        names: &[&'static str],
        on_button_clicked: Box<SpriteTabButtonClicked<S>>,
        on_highlight_changed: Option<Box<SpriteTabHighlightChanged<S>>>,
    ) -> anyhow::Result<Self> {
        let mut buttons = HashMap::new();

        for name in names {
            let button_id = format!("{prefix}_{name}");
            let component = panel.parser_state.fetch_component_as(&button_id)?;

            let sprite_id = format!("{button_id}_sprite");
            let id = panel.parser_state.get_widget_id(&sprite_id)?;
            let sprite_w = panel
                .layout
                .state
                .widgets
                .get_as::<WidgetSprite>(id)
                .ok_or_else(|| {
                    anyhow::anyhow!("Element with id=\"{sprite_id}\" must be a <sprite>")
                })?;

            let sprite = sprite_w.get_content().ok_or_else(|| {
                anyhow::anyhow!("Element with id=\"{sprite_id}\" must have a valid src!")
            })?;

            let state = S::from_tab_key(name);

            buttons.insert(
                *name,
                Rc::new(SpriteTabButtonState {
                    sprite,
                    component,
                    state,
                }),
            );
        }

        let top_sprite_id = panel
            .parser_state
            .get_widget_id(&format!("top_{prefix}_sprite"))?;
        Ok(Self {
            buttons,
            active_button: None,
            top_sprite_id,
            on_highlight_changed,
            on_button_clicked: Some(on_button_clicked),
        })
    }

    fn change_highlight(&mut self, common: &mut CallbackDataCommon, key: &str) {
        if let Some(old) = self.active_button.take() {
            old.component.set_sticky_state(common, false);
        }
        let new = self.buttons.get_mut(key).unwrap();
        new.component.set_sticky_state(common, true);
        self.active_button = Some(new.clone());

        if let Some(highlight_changed) = self.on_highlight_changed.as_ref() {
            highlight_changed(common, &new.state);
        }

        // change top sprite
        if let Some(mut sprite) = common
            .state
            .widgets
            .get_as::<WidgetSprite>(self.top_sprite_id)
        {
            sprite.set_content(common.alterables, Some(new.sprite.clone()));
        }
    }

    pub fn button_clicked(
        &mut self,
        common: &mut CallbackDataCommon,
        key: &str,
    ) -> Box<ModifyOverlayTask> {
        self.change_highlight(common, key);

        let state = S::from_tab_key(key);
        self.on_button_clicked.as_ref().unwrap()(common, &state)
    }

    pub fn reset(&mut self, common: &mut CallbackDataCommon, state: &S) {
        let key = state.to_tab_key();
        self.change_highlight(common, key);
    }
}
