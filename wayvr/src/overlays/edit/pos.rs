use wgui::parser::Fetchable;
use wlx_common::{common::LeftRight, windowing::Positioning};

use crate::{
    overlays::edit::{
        EditModeWrapPanel,
        sprite_tab::{SpriteTabHandler, SpriteTabKey},
    },
    windowing::window,
};

static POS_NAMES: [&str; 6] = ["static", "anchored", "floating", "hmd", "hand_l", "hand_r"];

#[derive(Default)]
pub struct PosTabState {
    pos: Positioning,
    has_lerp: bool,
    has_align: bool,
}

impl From<Positioning> for PosTabState {
    fn from(value: Positioning) -> Self {
        Self {
            pos: value,
            has_lerp: false,
            has_align: false,
        }
    }
}

pub fn new_pos_tab_handler(
    panel: &mut EditModeWrapPanel,
) -> anyhow::Result<SpriteTabHandler<PosTabState>> {
    let interpolation_id = panel.parser_state.get_widget_id("pos_interpolation")?;
    let align_to_hmd_id = panel.parser_state.get_widget_id("pos_align_to_hmd")?;

    SpriteTabHandler::new(
        panel,
        "pos",
        &POS_NAMES,
        Box::new(|_common, state| {
            let positioning = state.pos;
            Box::new(move |app, owc| {
                let state = owc.active_state.as_mut().unwrap(); //want panic
                state.positioning = positioning;
                window::save_transform(state, app);
            })
        }),
        Some(Box::new(move |common, state| {
            common
                .alterables
                .set_widget_visible(interpolation_id, state.has_lerp);
            common
                .alterables
                .set_widget_visible(align_to_hmd_id, state.has_align);
        })),
    )
}

impl SpriteTabKey for PosTabState {
    fn to_tab_key(&self) -> &'static str {
        match self.pos {
            Positioning::Static => "static",
            Positioning::Anchored => "anchored",
            Positioning::Floating => "floating",
            Positioning::FollowHead { .. } => "hmd",
            Positioning::FollowHand {
                hand: LeftRight::Left,
                ..
            } => "hand_l",
            Positioning::FollowHand {
                hand: LeftRight::Right,
                ..
            } => "hand_r",
        }
    }

    fn from_tab_key(key: &str) -> Self {
        match key {
            "static" => Self {
                pos: Positioning::Static,
                has_lerp: false,
                has_align: false,
            },
            "anchored" => Self {
                pos: Positioning::Anchored,
                has_lerp: false,
                has_align: false,
            },
            "floating" => Self {
                pos: Positioning::Floating,
                has_lerp: false,
                has_align: false,
            },
            "hmd" => Self {
                pos: Positioning::FollowHead { lerp: 1.0 },
                has_lerp: true,
                has_align: false,
            },
            "hand_l" => Self {
                pos: Positioning::FollowHand {
                    hand: LeftRight::Left,
                    lerp: 1.0,
                    align_to_hmd: false,
                },
                has_lerp: true,
                has_align: true,
            },
            "hand_r" => Self {
                pos: Positioning::FollowHand {
                    hand: LeftRight::Right,
                    lerp: 1.0,
                    align_to_hmd: false,
                },
                has_lerp: true,
                has_align: true,
            },
            _ => {
                panic!("cannot translate to positioning: {key}")
            }
        }
    }
}
