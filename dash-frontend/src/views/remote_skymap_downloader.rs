use std::{collections::HashMap, rc::Rc};

use crate::{
	frontend::{FrontendTask, FrontendTasks},
	util::{
		networking::{self, skymap_catalog::SkymapResolution},
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
	pub preview_image: CustomGlyphData,
	pub preview_image_compressed: Rc<Vec<u8>>,
	pub on_updated_library: Rc<dyn Fn()>,
}

#[derive(Clone)]
enum Task {
	Refresh,
	ResolutionClicked(SkymapResolution),
	DownloadFinished,
	RunDownload(SkymapResolution),
	RemoveFile(SkymapResolution),
}

pub struct View {
	entry: networking::skymap_catalog::SkymapCatalogEntry,
	frontend_tasks: FrontendTasks,
	globals: WguiGlobals,
	tasks: Tasks<Task>,
	executor: AsyncExecutor,

	id_resolution_buttons: WidgetID,

	#[allow(dead_code)]
	parser_state: ParserState,

	popup_download: PopupHolder<views::download_file::View>,
	popup_dialog_box: PopupHolder<views::dialog_box::View>,

	preview_image_compressed: Rc<Vec<u8>>,
	on_updated_library: Rc<dyn Fn()>,
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
				Task::ResolutionClicked(resolution) => {
					self.resolution_clicked(resolution)?;
				}
				Task::Refresh => {
					self.refresh(par.layout)?;
				}
				Task::DownloadFinished => {
					self.download_finished()?;
				}
				Task::RunDownload(resolution) => {
					self.run_download(resolution)?;
				}
				Task::RemoveFile(resolution) => {
					self.remove_file(resolution)?;
				}
			}
		}

		self.popup_download.update(par)?;
		self.popup_dialog_box.update(par)?;
		Ok(())
	}
}

fn doc_params(globals: &WguiGlobals) -> ParseDocumentParams<'_> {
	ParseDocumentParams {
		globals: globals.clone(),
		path: AssetPath::BuiltIn("gui/view/remote_skymap_downloader.xml"),
		extra: Default::default(),
	}
}

impl View {
	pub fn new(par: Params) -> anyhow::Result<Self> {
		let tasks = Tasks::<Task>::new();

		let parser_state = wgui::parser::parse_from_assets(&doc_params(&par.globals), par.layout, par.parent_id)?;
		let id_resolution_buttons = parser_state.get_widget_id("resolution_buttons")?;

		let str_version = par.globals.i18n().translate("VERSION");
		let str_creation_date = par.globals.i18n().translate("CREATION_DATE");
		let str_modification_date = par.globals.i18n().translate("MODIFICATION_DATE");

		let image = parser_state.fetch_widget(&par.layout.state, "image")?.widget;
		let mut image = image.cast::<WidgetImage>()?;
		image.set_content(&mut par.layout.alterables, Some(par.preview_image));

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
			tasks,
			globals: par.globals.clone(),
			executor: par.executor.clone(),
			entry: par.entry,
			parser_state,
			frontend_tasks: par.frontend_tasks,
			popup_download: Default::default(),
			popup_dialog_box: Default::default(),
			id_resolution_buttons,
			preview_image_compressed: par.preview_image_compressed,
			on_updated_library: par.on_updated_library,
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
				self.entry.is_downloaded(res)?,
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

	fn resolution_clicked(&mut self, resolution: SkymapResolution) -> anyhow::Result<()> {
		let is_downloaded = self.entry.is_downloaded(resolution).unwrap_or(false);
		if !is_downloaded {
			self.tasks.push(Task::RunDownload(resolution));
		} else {
			self.show_dialog_box_action(resolution)?;
		}
		Ok(())
	}

	fn show_dialog_box_action(&mut self, resolution: SkymapResolution) -> anyhow::Result<()> {
		const ACTION_REMOVE: &'static str = "remove";
		const ACTION_DOWNLOAD_AGAIN: &'static str = "download_again";

		let tasks = self.tasks.clone();

		views::dialog_box::mount_popup(
			self.popup_dialog_box.clone(),
			self.frontend_tasks.clone(),
			views::dialog_box::Params {
				globals: self.globals.clone(),
				message: Translation::from_translation_key("APP_SETTINGS.SKYMAP_ALREADY_DOWNLOADED"),
				entries: vec![
					views::dialog_box::ButtonEntry {
						content: Translation::from_translation_key("REMOVE"),
						icon: "dashboard/trash.svg",
						action: ACTION_REMOVE,
					},
					views::dialog_box::ButtonEntry {
						content: Translation::from_translation_key("DOWNLOAD_AGAIN"),
						icon: "dashboard/download.svg",
						action: ACTION_DOWNLOAD_AGAIN,
					},
				],
				on_action_click: Box::new(move |action| match action {
					ACTION_REMOVE => {
						tasks.push(Task::RemoveFile(resolution));
						tasks.push(Task::Refresh);
					}
					ACTION_DOWNLOAD_AGAIN => {
						tasks.push(Task::RunDownload(resolution));
						tasks.push(Task::Refresh);
					}
					_ => unreachable!(),
				}),
			},
		);

		Ok(())
	}

	fn download_finished(&mut self) -> anyhow::Result<()> {
		self.entry.save_metadata()?;
		let mut uuids = config_io::get_skymaps_uuids().unwrap_or_default();
		let uuid_str = self.entry.uuid.to_string();
		if !uuids.contains(&uuid_str) {
			uuids.push(uuid_str);
		}
		config_io::set_skymaps_uuids(&uuids)?;

		// Save preview image
		self.entry.files.save_preview_to_file(&self.preview_image_compressed)?;

		(*self.on_updated_library)();

		Ok(())
	}

	fn run_download(&mut self, resolution: SkymapResolution) -> anyhow::Result<()> {
		let Some(url) = self.entry.files.get_url_from_res(resolution) else {
			return Ok(());
		};

		let Some(target_path) = self.entry.get_destination_path(resolution) else {
			return Ok(());
		};

		views::download_file::mount_popup(
			self.popup_download.clone(),
			self.frontend_tasks.clone(),
			self.tasks.make_callback_box(Task::Refresh),
			views::download_file::Params {
				globals: self.globals.clone(),
				executor: self.executor.clone(),
				target_path,
				url,
				on_downloaded: self.tasks.make_callback_box(Task::DownloadFinished),
			},
		);
		Ok(())
	}

	fn remove_file(&mut self, resolution: SkymapResolution) -> anyhow::Result<()> {
		self.entry.remove_file(resolution);

		if !self.entry.has_any_downloaded() {
			// all skymaps of this uuid are removed, clean-up files
			self.entry.remove_metadata();

			// remove uuid of this entry from downloaded skymaps uuid and save the file again
			let mut uuids = config_io::get_skymaps_uuids().unwrap_or_default();
			uuids.retain(|uuid| *uuid != self.entry.uuid.to_string());
			config_io::set_skymaps_uuids(&uuids)?;

			// remove "_preview.dds" files from the disk too
			self.entry.files.remove_preview_file();
		}

		(*self.on_updated_library)();

		Ok(())
	}
}

pub fn mount_popup(
	frontend_tasks: FrontendTasks,
	executor: AsyncExecutor,
	globals: WguiGlobals,
	entry: networking::skymap_catalog::SkymapCatalogEntry,
	preview_image: CustomGlyphData,
	preview_image_compressed: Rc<Vec<u8>>,
	on_updated_library: Rc<dyn Fn()>,
	popup: PopupHolder<View>,
) {
	frontend_tasks
		.clone()
		.push(FrontendTask::MountPopupOnce(MountPopupOnceParams::new(
			Translation::from_raw_text(&entry.name),
			Box::new(move |data| {
				let view = View::new(Params {
					globals: &globals,
					layout: data.layout,
					executor: &executor,
					parent_id: data.id_content,
					entry,
					preview_image,
					frontend_tasks: frontend_tasks.clone(),
					preview_image_compressed,
					on_updated_library,
				})?;

				popup.set_view(data.handle, view, None);
				Ok(popup.get_close_callback(data.layout))
			}),
		)));
}
