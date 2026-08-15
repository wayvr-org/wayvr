use crate::{
	tab::settings::{
		SettingsMountParams, SettingsTab, UpdateExtra,
		macros::{options_category, options_stat_row},
	},
	views::ViewUpdateParams,
};
use std::f32::consts::TAU;
use wgui::{i18n::Translation, layout::WidgetID, widget::label::WidgetLabel};

pub struct State {
	id_label_rotations: WidgetID,
	id_label_session_time: WidgetID,
	id_label_lens_separation: WidgetID,
	last_rotations: f32,
	last_session_time_ms: u64,
	last_ipd: f32,
}

impl SettingsTab for State {
	fn update(&mut self, par: &mut ViewUpdateParams, extra: &UpdateExtra) -> anyhow::Result<()> {
		let hmd_stats = &extra.hmd_stats;

		let mut common = par.layout.common();

		// 1 - one rotation, 2 - two rotations
		// accuracy up to 0.01 rotations
		let rotations = (hmd_stats.rotations_rad / TAU * 100.0).round() / 100.0;

		if rotations != self.last_rotations {
			self.last_rotations = rotations;

			let mut label = common.state.widgets.cast_as::<WidgetLabel>(self.id_label_rotations)?;
			label.set_text(&mut common, Translation::from_raw_text_string(rotations.to_string()));
		}

		if hmd_stats.session_time_ms != self.last_session_time_ms {
			self.last_session_time_ms = hmd_stats.session_time_ms;

			let mut label = common
				.state
				.widgets
				.cast_as::<WidgetLabel>(self.id_label_session_time)?;
			label.set_text(
				&mut common,
				Translation::from_raw_text_string(format_duration(hmd_stats.session_time_ms)),
			);
		}

		if hmd_stats.ipd != self.last_ipd {
			self.last_ipd = hmd_stats.ipd;

			let mut label = common
				.state
				.widgets
				.cast_as::<WidgetLabel>(self.id_label_lens_separation)?;
			label.set_text(
				&mut common,
				Translation::from_raw_text_string(format!("{:.2}", hmd_stats.ipd)),
			);
		}

		Ok(())
	}
}

fn format_duration(ms: u64) -> String {
	let total_secs = ms / 1000;
	let h = total_secs / 3600;
	let m = (total_secs % 3600) / 60;
	let s = total_secs % 60;

	if h > 0 {
		format!("{h}:{m:02}:{s:02}")
	} else {
		format!("{m}:{s:02}")
	}
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<State> {
		let c = options_category(
			par.mp,
			par.id_parent,
			"APP_SETTINGS.STATISTICS",
			"dashboard/barchart.svg",
		)?;

		let id_label_rotations = options_stat_row(par.mp, c, "APP_SETTINGS.ROTATIONS")?;
		let id_label_session_time = options_stat_row(par.mp, c, "APP_SETTINGS.SESSION_TIME")?;
		let id_label_lens_separation = options_stat_row(par.mp, c, "APP_SETTINGS.LENS_SEPARATION_MM")?;

		Ok(State {
			id_label_rotations,
			id_label_session_time,
			id_label_lens_separation,
			last_rotations: f32::MIN,
			last_session_time_ms: u64::MAX,
			last_ipd: f32::MIN,
		})
	}
}
