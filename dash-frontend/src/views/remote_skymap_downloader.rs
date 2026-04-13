use std::{collections::HashMap, path::PathBuf, rc::Rc};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::{
		networking::{
			self,
			skymap_catalog::{SkymapCatalogEntry, SkymapResolution},
		},
		popup_manager::{MountPopupOnceParams, PopupHolder},
	},
	views::{self, ViewTrait, ViewUpdateParams},
};
use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	drawing::Color,
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	renderer_vk::text::custom_glyph::CustomGlyphData,
	task::Tasks,
	widget::{image::WidgetImage, label::WidgetLabel},
};
use wlx_common::{async_executor::AsyncExecutor, config_io};

pub struct Params<'a> {
	pub globals: &'a WguiGlobals,
	pub layout: &'a mut Layout,
	pub executor: &'a AsyncExecutor,
	pub frontend_tasks: FrontendTasks,
	pub parent_id: WidgetID,
	pub entry: networking::skymap_catalog::SkymapCatalogEntry,
	pub on_close_request: Box<dyn FnOnce()>,
	pub preview_image: Option<CustomGlyphData>,
}

#[derive(Clone)]
enum Task {
	Refresh,
	ResolutionClicked(networking::skymap_catalog::SkymapResolution),
}

pub struct View {
	id_parent: WidgetID,
	entry: networking::skymap_catalog::SkymapCatalogEntry,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	executor: AsyncExecutor,

	id_resolution_buttons: WidgetID,

	#[allow(dead_code)]
	parser_state: ParserState,

	popup_download: PopupHolder<views::download_file::View>,
}

fn mount_resolution_button(
	layout: &mut Layout,
	parser_state: &mut ParserState,
	doc_params: &ParseDocumentParams,
	parent_id: WidgetID,
	res: SkymapResolution,
	tasks: &Tasks<Task>,
	already_downloaded: bool,
) -> anyhow::Result<()> {
	let mut t = HashMap::<Rc<str>, Rc<str>>::new();
	t.insert(Rc::from("text"), Rc::from(res.get_display_str()));
	t.insert(
		Rc::from("sprite"),
		Rc::from(match already_downloaded {
			true => "dashboard/check.svg",
			false => "dashboard/download.svg",
		}),
	);
	let data = parser_state.realize_template(doc_params, "ResolutionButton", layout, parent_id, t)?;
	let button = data.fetch_component_as::<ComponentButton>("button")?;

	if already_downloaded {
		button.set_color(&mut layout.common(), Color::new(0.0, 0.4, 0.0, 1.0)); // green
	}
	tasks.handle_button(&button, Task::ResolutionClicked(res));
	Ok(())
}

impl ViewTrait for View {
	fn update(&mut self, par: &mut ViewUpdateParams) -> anyhow::Result<()> {
		for task in self.tasks.drain() {
			match task {
				Task::ResolutionClicked(skymap_resolution) => {
					self.run_download(skymap_resolution)?;
				}
				Task::Refresh => {
					self.refresh(par.layout)?;
				}
			}
		}

		self.popup_download.update(par)?;
		Ok(())
	}
}

fn get_skymap_resolution_full_path(entry: &SkymapCatalogEntry, resolution: SkymapResolution) -> Option<PathBuf> {
	let Some(filename) = entry.files.get_filename_from_res(resolution) else {
		return None;
	};

	Some(config_io::get_skymaps_root().join(filename))
}

fn is_downloaded(entry: &SkymapCatalogEntry, resolution: SkymapResolution) -> anyhow::Result<bool> {
	let Some(full_path) = get_skymap_resolution_full_path(entry, resolution) else {
		return Ok(false);
	};

	Ok(std::fs::exists(full_path)?)
}

fn doc_params(globals: &WguiGlobals) -> ParseDocumentParams {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/view/remote_skymap_downloader.xml"),
		extra: Default::default(),
	}
}

impl View {
	pub fn new(par: Params) -> anyhow::Result<Self> {
		let tasks = Tasks::<Task>::new();

		let mut parser_state = wgui::parser::parse_from_assets(&doc_params(&par.globals), par.layout, par.parent_id)?;
		let id_resolution_buttons = parser_state.get_widget_id("resolution_buttons")?;

		let str_version = par.globals.i18n().translate("VERSION");
		let str_creation_date = par.globals.i18n().translate("CREATION_DATE");
		let str_modification_date = par.globals.i18n().translate("MODIFICATION_DATE");

		let image = parser_state.fetch_widget(&par.layout.state, "image")?.widget;
		let mut image = image.cast::<WidgetImage>()?;
		image.set_content(&mut par.layout.alterables, par.preview_image);

		// Set author label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_author")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text_string(format!("by {}", par.entry.author)),
			);

		// Set description label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_description")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text(&par.entry.description),
			);

		// Set version label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_version")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text_string(format!("{}: {}", str_version, par.entry.version)),
			);

		// Set creation date label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_creation_date")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text_string(format!("{}: {}", str_creation_date, par.entry.created_at)),
			);

		// Set modification date label
		parser_state
			.fetch_widget_as::<WidgetLabel>(&par.layout.state, "label_modification_date")?
			.set_text_simple(
				&mut par.globals.get(),
				Translation::from_raw_text_string(format!("{}: {}", str_modification_date, par.entry.created_at)),
			);

		tasks.push(Task::Refresh);

		Ok(Self {
			id_parent: par.parent_id,
			tasks,
			globals: par.globals.clone(),
			executor: par.executor.clone(),
			entry: par.entry,
			parser_state,
			frontend_tasks: par.frontend_tasks,
			popup_download: Default::default(),
			id_resolution_buttons,
		})
	}

	fn refresh(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		layout.remove_children(self.id_resolution_buttons);

		let files = &self.entry.files;
		let mut mount_res = |res: SkymapResolution| -> anyhow::Result<()> {
			mount_resolution_button(
				layout,
				&mut self.parser_state,
				&doc_params(&self.globals),
				self.id_resolution_buttons,
				res,
				&self.tasks,
				is_downloaded(&self.entry, res)?,
			)
		};

		mount_res(SkymapResolution::Res2k)?;
		if files.size_4k.is_some() {
			mount_res(SkymapResolution::Res4k)?;
		}
		if files.size_8k.is_some() {
			mount_res(SkymapResolution::Res8k)?;
		}
		Ok(())
	}

	fn run_download(&mut self, resolution: SkymapResolution) -> anyhow::Result<()> {
		let Some(url) = self.entry.files.get_url_from_res(resolution) else {
			return Ok(());
		};

		let Some(full_path) = get_skymap_resolution_full_path(&self.entry, resolution) else {
			return Ok(());
		};

		views::download_file::mount_popup(
			self.frontend_tasks.clone(),
			self.executor.clone(),
			self.globals.clone(),
			self.popup_download.clone(),
			full_path,
			url,
			self.tasks.make_callback_box(Task::Refresh),
		);
		Ok(())
	}
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	executor: AsyncExecutor,
	globals: WguiGlobals,
	entry: networking::skymap_catalog::SkymapCatalogEntry,
	preview_image: Option<CustomGlyphData>,
	popup: PopupHolder<View>,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_raw_text(&entry.name),
			Box::new(move |data| {
				let on_close_request = popup.get_close_callback(data.layout);
				let view = View::new(Params {
					globals: &globals,
					layout: data.layout,
					executor: &executor,
					parent_id: data.id_content,
					entry,
					on_close_request,
					preview_image,
					frontend_tasks: frontend_tasks.clone(),
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}
