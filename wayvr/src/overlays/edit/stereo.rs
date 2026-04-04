use crate::overlays::edit::{
    EditModeWrapPanel,
    sprite_tab::{SpriteTabHandler, SpriteTabKey},
};
use wgui::{components::checkbox::ComponentCheckbox, i18n::Translation, parser::Fetchable};
use wlx_common::overlays::{BackendAttribValue, StereoMode};

static STEREO_NAMES: [&str; 5] = ["none", "leftright", "rightleft", "topbottom", "bottomtop"];

pub fn new_stereo_tab_handler(
    panel: &mut EditModeWrapPanel,
) -> anyhow::Result<SpriteTabHandler<StereoMode>> {
    // Fetch the checkbox component first, before creating the closure
    let checkbox = panel
        .parser_state
        .fetch_component_as::<ComponentCheckbox>("stereo_full_frame_box")?;

    SpriteTabHandler::new(
        panel,
        "stereo",
        &STEREO_NAMES,
        Box::new(move |common, state| {
            let stereo = *state;

            let translation = get_stereo_full_frame_translation(stereo);
            checkbox.set_text(common, Translation::from_translation_key(translation));

            Box::new(move |app, owc| {
                owc.backend
                    .set_attrib(app, BackendAttribValue::Stereo(stereo));
            })
        }),
        None,
    )
}

impl SpriteTabKey for StereoMode {
    fn to_tab_key(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::LeftRight => "leftright",
            Self::RightLeft => "rightleft",
            Self::TopBottom => "topbottom",
            Self::BottomTop => "bottomtop",
        }
    }

    fn from_tab_key(key: &str) -> Self {
        match key {
            "none" => Self::None,
            "leftright" => Self::LeftRight,
            "rightleft" => Self::RightLeft,
            "topbottom" => Self::TopBottom,
            "bottomtop" => Self::BottomTop,
            _ => {
                panic!("cannot translate to stereo mode: {key}")
            }
        }
    }
}

pub const fn get_stereo_full_frame_translation(stereo: StereoMode) -> &'static str {
    match stereo {
        StereoMode::LeftRight | StereoMode::RightLeft => "EDIT_MODE.STEREO_3D_MODE.FULL_FRAME_SBS",
        StereoMode::TopBottom => "EDIT_MODE.STEREO_3D_MODE.FULL_FRAME_TAB",
        StereoMode::BottomTop => "EDIT_MODE.STEREO_3D_MODE.FULL_FRAME_BAT",
        StereoMode::None => "EDIT_MODE.STEREO_3D_MODE.FULL_FRAME",
    }
}
