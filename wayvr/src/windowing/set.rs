use slotmap::SecondaryMap;
use std::sync::Arc;
use wlx_common::{astr_containers::AStrMap, windowing::OverlayWindowState};

use crate::windowing::OverlayID;

#[derive(Default)]
pub struct OverlayWindowSet {
    pub(super) name: Arc<str>,
    pub(super) overlays: SecondaryMap<OverlayID, OverlayWindowState>,

    /// overlays that are not currently visible
    pub(super) hidden_overlays: AStrMap<OverlayWindowState>,

    /// overlays that have not been seen since startup
    pub(super) inactive_overlays: AStrMap<OverlayWindowState>,
}

impl OverlayWindowSet {
    pub(super) fn copied_from(src: &Self) -> Self {
        Self {
            name: Arc::from(""),
            overlays: src.overlays.clone(),
            hidden_overlays: src.hidden_overlays.clone(),
            inactive_overlays: src.inactive_overlays.clone(),
        }
    }
}
