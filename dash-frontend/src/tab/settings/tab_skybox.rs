use crate::{
	tab::settings::{
		SettingType, SettingsMountParams, SettingsTab,
		macros::{options_category, options_checkbox},
	},
	views::skymap_list,
};

pub struct State {
	skymap_list: skymap_list::View,
}

impl SettingsTab for State {
	fn update(&mut self, par: super::SettingsUpdateParams) -> anyhow::Result<()> {
		self.skymap_list.update(par.layout, par.executor)?;
		Ok(())
	}
}

impl State {
	pub fn mount(par: SettingsMountParams) -> anyhow::Result<Self> {
		let c = options_category(par.mp, par.parent_id, "APP_SETTINGS.SKYBOX", "dashboard/globe.svg")?;
		options_checkbox(par.mp, c, SettingType::UseSkybox)?;
		options_checkbox(par.mp, c, SettingType::OpaqueBackground)?;

		let skymap_list = skymap_list::View::new(skymap_list::Params {
			globals: par.globals.clone(),
			layout: par.mp.layout,
			parent_id: c,
			frontend_tasks: par.frontend_tasks,
		})?;

		Ok(Self { skymap_list })
	}
}
