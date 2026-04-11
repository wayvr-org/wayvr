use std::{
	cell::RefCell,
	rc::{Rc, Weak},
};

use wgui::{
	assets::AssetPath,
	components::button::ComponentButton,
	event::{EventAlterables, StyleSetRequest},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, LayoutTask, LayoutTasks, WidgetID},
	parser::{Fetchable, ParseDocumentParams, ParserState},
	taffy::Display,
	widget::label::WidgetLabel,
};
use wlx_common::config::GeneralConfig;

use crate::frontend::{FrontendTask, FrontendTasks};

pub struct PopupManagerParams {
	pub parent_id: WidgetID,
}

struct State {
	popup_stack: Vec<Weak<RefCell<MountedPopupState>>>,
}

pub struct MountedPopup {
	#[allow(dead_code)]
	state: ParserState,
	id_root: WidgetID, // decorations of a popup
	layout_tasks: LayoutTasks,
	frontend_tasks: FrontendTasks,
}

#[derive(Default)]
struct MountedPopupState {
	mounted_popup: Option<MountedPopup>,
}

#[derive(Default, Clone)]
pub struct PopupHandle {
	state: Rc<RefCell<MountedPopupState>>,
}

struct PopupHolderState<ViewType> {
	popup_handle: PopupHandle,
	view: Option<ViewType>,
}

// we can't use #[derive(Default)] due to the fact that ViewType can't be Default.
impl<ViewType> Default for PopupHolderState<ViewType> {
	fn default() -> Self {
		Self {
			popup_handle: Default::default(),
			view: Default::default(),
		}
	}
}

pub struct PopupHolder<ViewType> {
	state: Rc<RefCell<PopupHolderState<ViewType>>>,
}

impl<ViewType> Default for PopupHolder<ViewType> {
	fn default() -> Self {
		Self {
			state: Rc::new(RefCell::new(PopupHolderState::default())),
		}
	}
}

// we can't derive(Clone) due to the fact that ViewType is non-cloneable
impl<ViewType> Clone for PopupHolder<ViewType> {
	fn clone(&self) -> Self {
		Self {
			state: self.state.clone(),
		}
	}
}

impl<ViewType> PopupHolder<ViewType> {
	pub fn close(&self) {
		let mut state = self.state.borrow_mut();
		state.view = None;
		state.popup_handle.close();
	}

	pub fn set_view(&self, handle: PopupHandle, view: ViewType) {
		let mut state = self.state.borrow_mut();
		state.view = Some(view);
		state.popup_handle = handle;
	}

	// Get underlying ViewType object in a closure and return its value
	// example usage:
	//
	// ```rs
	// holder.with_view(|view| {
	//   view.foo();
	// })
	// ```
	//
	pub fn with_view<F, R>(&self, f: F) -> Option<R>
	where
		F: FnOnce(&mut ViewType) -> R,
	{
		let mut state = self.state.borrow_mut();
		if let Some(view) = state.view.as_mut() {
			Some(f(view))
		} else {
			None
		}
	}

	// Same as with_view, but the closure expects a simple anyhow::Result<()> type
	pub fn with_view_res<F>(&self, f: F) -> anyhow::Result<()>
	where
		F: FnOnce(&mut ViewType) -> anyhow::Result<()>,
	{
		if let Some(res) = self.with_view(f) {
			return res;
		}
		Ok(())
	}

	pub fn get_close_callback(&self) -> Box<dyn Fn()>
	where
		ViewType: 'static,
	{
		let this = self.clone();
		Box::new(move || this.close())
	}
}

impl PopupHandle {
	pub fn close(&self) {
		self.state.borrow_mut().mounted_popup = None; // Drop will be called
	}
}

pub struct PopupManager {
	state: Rc<RefCell<State>>,
	parent_id: WidgetID,
}

pub struct PopupContentFuncData<'a> {
	pub layout: &'a mut Layout,
	pub config: &'a GeneralConfig,
	pub handle: PopupHandle,
	pub id_content: WidgetID,
}

#[derive(Clone)]
pub struct MountPopupParams {
	pub title: Translation,
	pub on_content: Rc<dyn Fn(PopupContentFuncData) -> anyhow::Result<()>>,
}

// we need to implement Clone here, but the underlying function can be called only once.
// on_content will be cleared after the first call
#[derive(Clone)]
pub struct MountPopupOnceParams {
	title: Translation,
	on_content: Rc<RefCell<Option<Box<dyn FnOnce(PopupContentFuncData) -> anyhow::Result<()>>>>>,
}

impl MountPopupOnceParams {
	pub fn new(title: Translation, on_content: Box<dyn FnOnce(PopupContentFuncData) -> anyhow::Result<()>>) -> Self {
		Self {
			title,
			on_content: Rc::new(RefCell::new(Some(on_content))),
		}
	}
}

impl Drop for MountedPopup {
	fn drop(&mut self) {
		self.layout_tasks.push(LayoutTask::RemoveWidget(self.id_root));
		self.frontend_tasks.push(FrontendTask::RefreshPopupManager);
	}
}

impl State {
	fn refresh_stack(&mut self, alterables: &mut EventAlterables) {
		// show only the topmost popup
		self.popup_stack.retain(|weak| {
			let Some(popup) = weak.upgrade() else {
				return false;
			};
			popup.borrow_mut().mounted_popup.is_some()
		});

		for (idx, popup) in self.popup_stack.iter().enumerate() {
			let popup = popup.upgrade().unwrap(); // safe
			let popup = popup.borrow_mut();
			let mounted_popup = popup.mounted_popup.as_ref().unwrap(); // safe;

			alterables.set_style(
				mounted_popup.id_root,
				StyleSetRequest::Display(if idx == self.popup_stack.len() - 1 {
					Display::Flex
				} else {
					Display::None
				}),
			);
		}
	}
}

impl PopupManager {
	pub fn new(params: PopupManagerParams) -> Self {
		Self {
			parent_id: params.parent_id,
			state: Rc::new(RefCell::new(State {
				popup_stack: Vec::new(),
			})),
		}
	}

	pub fn refresh(&self, alterables: &mut EventAlterables) {
		let mut state = self.state.borrow_mut();
		state.refresh_stack(alterables);
	}

	fn mount_popup_prepare(
		&self,
		globals: &WguiGlobals,
		layout: &mut Layout,
		frontend_tasks: &FrontendTasks,
		popup_title: &Translation,
	) -> anyhow::Result<(PopupHandle, WidgetID /* content widget ID */)> {
		let doc_params = &ParseDocumentParams {
			globals: globals.clone(),
			path: AssetPath::BuiltIn("gui/view/popup_window.xml"),
			extra: Default::default(),
		};
		let state = wgui::parser::parse_from_assets(doc_params, layout, self.parent_id)?;

		let id_root = state.get_widget_id("root")?;
		let id_content = state.get_widget_id("content")?;

		{
			let mut label_title = state.fetch_widget_as::<WidgetLabel>(&layout.state, "popup_title")?;
			label_title.set_text_simple(&mut globals.get(), popup_title.clone());
		}

		let but_back = state.fetch_component_as::<ComponentButton>("but_back")?;

		let mounted_popup = MountedPopup {
			state,
			id_root,
			layout_tasks: layout.tasks.clone(),
			frontend_tasks: frontend_tasks.clone(),
		};

		let mounted_popup_state = MountedPopupState {
			mounted_popup: Some(mounted_popup),
		};

		let popup_handle = PopupHandle {
			state: Rc::new(RefCell::new(mounted_popup_state)),
		};

		let mut state = self.state.borrow_mut();
		state.popup_stack.push(Rc::downgrade(&popup_handle.state));

		but_back.on_click({
			let popup_handle = Rc::downgrade(&popup_handle.state);
			Rc::new(move |_common, _evt| {
				if let Some(popup_handle) = popup_handle.upgrade() {
					popup_handle.borrow_mut().mounted_popup = None; // will call Drop
				}
				Ok(())
			})
		});

		frontend_tasks.push(FrontendTask::RefreshPopupManager);
		Ok((popup_handle, id_content))
	}

	/// Mount a new popup on top of the existing popup stack (non-cloneable version).
	/// Only the topmost popup is visible.
	pub fn mount_popup_once(
		&mut self,
		globals: &WguiGlobals,
		layout: &mut Layout,
		frontend_tasks: &FrontendTasks,
		params: MountPopupOnceParams,
		config: &GeneralConfig,
	) -> anyhow::Result<()> {
		let mut func = params.on_content.borrow_mut();
		let Some(on_content_func) = func.take() else {
			anyhow::bail!("mount_popup_once called more than once");
		};

		let (popup_handle, id_content) = self.mount_popup_prepare(globals, layout, frontend_tasks, &params.title)?;

		// mount user-set popup content
		on_content_func(PopupContentFuncData {
			layout,
			handle: popup_handle.clone(),
			id_content,
			config,
		})?;

		Ok(())
	}

	/// Mount a new popup on top of the existing popup stack.
	/// Only the topmost popup is visible.
	pub fn mount_popup(
		&mut self,
		globals: &WguiGlobals,
		layout: &mut Layout,
		frontend_tasks: &FrontendTasks,
		params: MountPopupParams,
		config: &GeneralConfig,
	) -> anyhow::Result<()> {
		let (popup_handle, id_content) = self.mount_popup_prepare(globals, layout, frontend_tasks, &params.title)?;

		// mount user-set popup content
		(*params.on_content)(PopupContentFuncData {
			layout,
			handle: popup_handle.clone(),
			id_content,
			config,
		})?;

		Ok(())
	}
}
