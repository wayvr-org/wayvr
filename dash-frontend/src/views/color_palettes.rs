use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::popup_manager::{MountPopupOnceParams, PopupHolder},
	views::{ViewTrait, ViewUpdateParams},
};
use std::{rc::Rc, sync::Arc};
use wgui::color::WguiColorName;
use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, LayoutTask, WidgetID},
	log::LogErr,
	palette::PALETTES,
	parser::{Fetchable, ParseDocumentParams, TemplateParams},
	task::Tasks,
};
use wlx_common::{
	dash_interface::ConfigChangeKind,
	palette::{list_palette_files, load_custom_palette},
};

#[derive(Clone)]
enum Task {
	SelectPalette(String),
	CustomPaletteUrl,
}

pub struct Params<'a> {
	pub globals: WguiGlobals,
	pub layout: &'a mut Layout,
	pub parent_id: WidgetID,
	pub frontend_tasks: &'a FrontendTasks,
	pub on_close_request: Box<dyn FnOnce()>,
	pub current_palette: Arc<str>,
}

pub struct View {
	tasks: Tasks<Task>,
	frontend_tasks: FrontendTasks,
	on_close_request: Option<Box<dyn FnOnce()>>,
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::SelectPalette(profile) => {
					let mut globals = par.layout.state.globals.get();
					let new_palette = wlx_common::palette::load_palette(&profile);
					globals.palette = new_palette;
					par.general_config.color_palette = profile.into();
					par.layout.tasks.push(LayoutTask::RefreshPalette);
					par.config_change_kind.replace(ConfigChangeKind::WguiColorPaletteChange);
					if let Some(c) = self.on_close_request.take() {
						c();
					}
				}
				Task::CustomPaletteUrl => {
					self.frontend_tasks.push(FrontendTask::OpenURL(
						"https://wayvr.org/docs/basics/customization/".into(),
					));
				}
			}
		}
		Ok(())
	}
}

macro_rules! insert_colors {
	(
		$params:expr,
		$palette:expr,
		$( $key:literal => $color:ident ),* $(,)?
	) => {
		$(
			$params.insert_str(
				$key,
				WguiColorName::$color
					.to_wgui_color()
					.resolve($palette)
					.to_hex()
			);
		)*
	};
}

impl View {
	pub fn new(params: Params) -> anyhow::Result<Self> {
		let doc_params = &ParseDocumentParams {
			globals: params.globals.clone(),
			path: AssetPath::BuiltIn("gui/view/color_palettes.xml"),
			extra: Default::default(),
		};

		let mut parser_state = wgui::parser::parse_from_assets(doc_params, params.layout, params.parent_id)?;

		let list_parent = parser_state.fetch_widget(&params.layout.state, "list_parent")?.id;

		let tasks = Tasks::new();

		for (idx, name) in list_palette_files().into_iter().enumerate() {
			let Ok(palette) = load_custom_palette(&name).log_warn("Could not load custom color palette") else {
				continue;
			};

			let id = format!("profile_custom_{idx}");
			let is_current = &*params.current_palette == name.as_str();

			let mut cell_params = TemplateParams::new();
			cell_params.insert("id", &id);

			let display_name = &name[..name.len() - 5];

			if is_current {
				cell_params.insert_str("text", format!("{display_name} ✅"));
				cell_params.insert("tooltip", "APP_SETTINGS.COLOR_PALETTE_CURRENT");
			} else {
				cell_params.insert("text", display_name);
				cell_params.insert("tooltip", "APP_SETTINGS.COLOR_PALETTE_ACTIVATE");
			}

			insert_colors!(
				cell_params,
				&palette,
				"primary" => Primary,
				"on_primary" => OnPrimary,
				"secondary" => Secondary,
				"on_secondary" => OnSecondary,
				"tertiary" => Tertiary,
				"on_tertiary" => OnTertiary,
				"danger" => Danger,
				"on_danger" => OnDanger,
				"background" => Background,
				"on_background" => OnBackground,
				"background_variant" => BackgroundVariant,
				"outline" => Outline,
				"highlight" => Highlight,
			);

			parser_state.instantiate_template(
				doc_params,
				"ColorPaletteButton",
				params.layout,
				list_parent,
				cell_params,
			)?;

			if !is_current {
				let btn = parser_state.fetch_component_as::<ComponentButton>(&id)?;
				let tasks_clone = tasks.clone();
				btn.on_click(Rc::new({
					move |_common, _e| {
						tasks_clone.push(Task::SelectPalette(name.to_string()));
						Ok(())
					}
				}));
			}
		}

		for (idx, (name, palette)) in PALETTES.iter().enumerate() {
			let id = format!("profile_builtin_{idx}");
			let is_current = &*params.current_palette == *name;

			let mut cell_params = TemplateParams::new();
			cell_params.insert("id", &id);

			if is_current {
				cell_params.insert_str("text", format!("{name} ✅"));
				cell_params.insert("tooltip", "APP_SETTINGS.COLOR_PALETTE_CURRENT");
			} else {
				cell_params.insert("text", name);
				cell_params.insert("tooltip", "APP_SETTINGS.COLOR_PALETTE_ACTIVATE");
			}

			insert_colors!(
				cell_params,
				palette,
				"primary" => Primary,
				"on_primary" => OnPrimary,
				"secondary" => Secondary,
				"on_secondary" => OnSecondary,
				"tertiary" => Tertiary,
				"on_tertiary" => OnTertiary,
				"danger" => Danger,
				"on_danger" => OnDanger,
				"background" => Background,
				"on_background" => OnBackground,
				"background_variant" => BackgroundVariant,
				"outline" => Outline,
				"highlight" => Highlight,
			);

			parser_state.instantiate_template(
				doc_params,
				"ColorPaletteButton",
				params.layout,
				list_parent,
				cell_params,
			)?;

			if !is_current {
				let btn = parser_state.fetch_component_as::<ComponentButton>(&id)?;
				let tasks_clone = tasks.clone();
				btn.on_click(Rc::new({
					move |_common, _e| {
						tasks_clone.push(Task::SelectPalette(name.to_string()));
						Ok(())
					}
				}));
			}
		}

		parser_state.instantiate_template(
			doc_params,
			"CustomPaletteButton",
			params.layout,
			list_parent,
			TemplateParams::default(),
		)?;
		let btn = parser_state.fetch_component_as::<ComponentButton>("custom_btn")?;
		let tasks_clone = tasks.clone();
		btn.on_click(Rc::new({
			move |_common, _e| {
				tasks_clone.push(Task::CustomPaletteUrl);
				Ok(())
			}
		}));

		Ok(Self {
			tasks,
			frontend_tasks: params.frontend_tasks.clone(),
			on_close_request: Some(params.on_close_request),
		})
	}
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	popup: PopupHolder<View>,
	current_palette: Arc<str>,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_translation_key("APP_SETTINGS.COLOR_PALETTES"),
			Box::new(move |data| {
				let on_close_request = popup.get_close_callback(data.layout);
				let view = View::new(Params {
					globals: globals.clone(),
					layout: data.layout,
					parent_id: data.id_content,
					frontend_tasks: &frontend_tasks,
					current_palette,
					on_close_request,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
			Default::default(), /* extra */
		)));
}
