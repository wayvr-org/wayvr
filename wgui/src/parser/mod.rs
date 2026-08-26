mod component_bar_graph;
mod component_button;
mod component_checkbox;
mod component_color_selector;
mod component_editbox;
mod component_radio_group;
mod component_slider;
mod component_tabs;

#[cfg(feature = "video")]
mod component_video;

mod helpers;
mod style;
mod widget_div;
mod widget_image;
mod widget_label;
mod widget_rectangle;
mod widget_sprite;

use crate::{
	assets::{AssetPathRc, AssetPathRef, AssetPathSource, normalize_path},
	components::{Component, ComponentWeak},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, LayoutParams, LayoutState, Widget, WidgetID, WidgetMap, WidgetPair},
	log::LogErr,
	parser::{
		component_bar_graph::parse_component_bar_graph,
		component_button::parse_component_button,
		component_checkbox::{CheckboxKind, parse_component_checkbox},
		component_color_selector::parse_component_color_selector,
		component_editbox::parse_component_editbox,
		component_radio_group::parse_component_radio_group,
		component_slider::parse_component_slider,
		component_tabs::parse_component_tabs,
		widget_div::parse_widget_div,
		widget_image::parse_widget_image,
		widget_label::parse_widget_label,
		widget_rectangle::parse_widget_rectangle,
		widget_sprite::parse_widget_sprite,
	},
	widget::ConstructEssentials,
	windowing::context_menu,
};
use anyhow::Context;
use ouroboros::self_referencing;
use smallvec::SmallVec;
use std::{
	cell::RefMut,
	collections::HashMap,
	path::{Path, PathBuf},
	rc::Rc,
};

#[self_referencing]
struct XmlDocument {
	xml: String,

	#[borrows(xml)]
	#[covariant]
	doc: roxmltree::Document<'this>,
}

pub struct Template {
	node_document: Rc<XmlDocument>,
	node: roxmltree::NodeId, // belongs to node_document which could be included in another file
	root_dir: AssetPathRc,
	xml_path_source: AssetPathSource,
	xml_path: AssetPathRc,
	xml_dir: AssetPathRc,
}

#[derive(Clone, Default)]
pub struct TemplateParams(HashMap<Rc<str>, Rc<str>>);

// Changelog:
// v1: source paths are always relative to the assets root
//
// v2: added "@/" prefix for paths relative to the assets root;
// bare paths (e.g. "./foo/bar" or "foo/bar" remain relative to the currently parsing XML file
//
#[derive(Clone, Copy)]
pub struct Version(u32);

struct ParserFile {
	version: Version,

	// used for path expansion, see expand_path fn.
	// For internal and builtin paths:           '@' always expands to '/`.
	// For xml files residing on the filesystem: '@' expands to the user-defined xml
	root_dir: AssetPathRc,

	// Where this xml file resides (internal/builtin/filesystem)
	// notice there's no FileOrBuiltIn, we've already resolved it here.
	xml_path_source: AssetPathSource,

	xml_path: AssetPathRc,
	xml_dir: AssetPathRc,
	document: Rc<XmlDocument>,
	template_parameters: TemplateParams,
}

/*
	`components` could contain connected listener handles.
		Do not drop them unless you don't need to handle any events,
		including mouse-hover animations.
*/
#[derive(Default, Clone)]
pub struct ParserData {
	pub components_by_id: HashMap<Rc<str>, ComponentWeak>,
	pub components: Vec<Component>,
	pub ids: HashMap<Rc<str>, WidgetID>,
	pub templates: HashMap<Rc<str>, Rc<Template>>,
	pub var_map: HashMap<Rc<str>, Rc<str>>,
	macro_attribs: HashMap<Rc<str>, MacroAttribs>,
}

pub trait Fetchable {
	/// Return a component by its string ID
	fn fetch_component_by_id(&self, id: &str) -> anyhow::Result<Component>;

	/// Fetch a component by widget ID (returns Component)
	fn fetch_component_by_widget_id(&self, state: &LayoutState, widget_id: WidgetID) -> anyhow::Result<Component>;

	/// Fetch a component by string ID and down‑cast it to a concrete component type `T` (see `components/mod.rs`)
	fn fetch_component_as<T: 'static>(&self, id: &str) -> anyhow::Result<Rc<T>>;

	/// Fetch a component by widget ID and down‑cast it to a concrete component type `T` (see `components/mod.rs`)
	fn fetch_component_from_widget_id_as<T: 'static>(
		&self,
		state: &LayoutState,
		widget_id: WidgetID,
	) -> anyhow::Result<Rc<T>>;

	/// Return a widget by its string ID
	fn get_widget_id(&self, id: &str) -> anyhow::Result<WidgetID>;

	/// Retrieve the widget associated with a string ID, returning a `WidgetPair` (id and widget itself)
	fn fetch_widget(&self, state: &LayoutState, id: &str) -> anyhow::Result<WidgetPair>;

	/// Retrieve a widget by string ID and down‑cast its inner value to type `T` (see `widget/mod.rs`)
	fn fetch_widget_as<'a, T: 'static>(&self, state: &'a LayoutState, id: &str) -> anyhow::Result<RefMut<'a, T>>;
}

impl TemplateParams {
	pub fn new() -> Self {
		Self(HashMap::new())
	}

	pub const fn from_hashmap(map: HashMap<Rc<str>, Rc<str>>) -> Self {
		Self(map)
	}

	pub fn insert(&mut self, key: &str, value: &str) -> Option<Rc<str>> {
		self.0.insert(Rc::from(key), Rc::from(value))
	}

	pub fn insert_rc(&mut self, key: &str, value: Rc<str>) -> Option<Rc<str>> {
		self.0.insert(Rc::from(key), value)
	}

	pub fn insert_str(&mut self, key: &str, value: String) -> Option<Rc<str>> {
		self.0.insert(Rc::from(key), value.into())
	}
}

impl ParserData {
	pub(crate) fn take_results_from(&mut self, from: &mut Self) {
		let ids = std::mem::take(&mut from.ids);
		let components = std::mem::take(&mut from.components);
		let components_by_id = std::mem::take(&mut from.components_by_id);

		for (id, key) in ids {
			self.ids.insert(id, key);
		}

		for c in components {
			self.components.push(c);
		}

		for (k, v) in components_by_id {
			self.components_by_id.insert(k, v);
		}
	}
}

impl Fetchable for ParserData {
	fn fetch_component_by_id(&self, id: &str) -> anyhow::Result<Component> {
		let Some(weak) = self.components_by_id.get(id) else {
			anyhow::bail!("Component by ID \"{id}\" doesn't exist");
		};

		let Some(component) = weak.upgrade() else {
			anyhow::bail!("Component by ID \"{id}\" doesn't exist");
		};

		Ok(Component(component))
	}

	fn fetch_component_by_widget_id(&self, state: &LayoutState, widget_id: WidgetID) -> anyhow::Result<Component> {
		state.fetch_component_by_widget_id(widget_id)
	}

	fn fetch_component_from_widget_id_as<T: 'static>(
		&self,
		state: &LayoutState,
		widget_id: WidgetID,
	) -> anyhow::Result<Rc<T>> {
		state.fetch_component_from_widget_id_as(widget_id)
	}

	fn fetch_component_as<T: 'static>(&self, id: &str) -> anyhow::Result<Rc<T>> {
		let component = self.fetch_component_by_id(id)?;

		if !(*component.0).as_any().is::<T>() {
			anyhow::bail!("fetch_component_as({id}): type not matching");
		}

		// safety: we just checked the type
		unsafe { Ok(Rc::from_raw(Rc::into_raw(component.0).cast())) }
	}

	fn get_widget_id(&self, id: &str) -> anyhow::Result<WidgetID> {
		match self.ids.get(id) {
			Some(id) => Ok(*id),
			None => anyhow::bail!("Widget by ID \"{id}\" doesn't exist"),
		}
	}

	fn fetch_widget(&self, state: &LayoutState, id: &str) -> anyhow::Result<WidgetPair> {
		let widget_id = self.get_widget_id(id)?;
		let widget = state
			.widgets
			.get(widget_id)
			.ok_or_else(|| anyhow::anyhow!("fetch_widget({id}): widget not found"))?;
		Ok(WidgetPair {
			id: widget_id,
			widget: widget.clone(),
		})
	}

	fn fetch_widget_as<'a, T: 'static>(&self, state: &'a LayoutState, id: &str) -> anyhow::Result<RefMut<'a, T>> {
		let widget_id = self.get_widget_id(id)?;
		let widget = state
			.widgets
			.get(widget_id)
			.ok_or_else(|| anyhow::anyhow!("fetch_widget_as({id}): widget not found"))?;

		let casted = widget
			.get_as::<T>()
			.ok_or_else(|| anyhow::anyhow!("fetch_widget_as({id}): failed to cast"))?;
		Ok(casted)
	}
}

/*
	WARNING: this struct could contain valid components with already bound listener handles.
	Make sure to store them somewhere in your code.
*/
pub struct ParserState {
	version: Version,
	xml_path_source: AssetPathSource,
	pub data: ParserData,
	pub root_dir: AssetPathRc,
	pub xml_path: AssetPathRc, // path of the currently processing xml file
	pub xml_dir: AssetPathRc,  // same as xml_path, but with stripped filename
}

impl ParserState {
	/// Parse named <template> tag and process it.
	/// Preferred method of parsing templates. Same as `parse_template_only`,
	/// but it keeps components data in this `ParserState` object for you.
	/// The result can be safely dropped, all required event listeners and components
	/// will be kept intact in this `ParserState`.
	/// Resulting `ParserData::components` Vec will be left empty (they are moved into this `ParserState::data`)
	pub fn realize_template(
		&mut self,
		doc_params: &ParseDocumentParams,
		template_name: &str,
		layout: &mut Layout,
		widget_id: WidgetID,
		template_parameters: TemplateParams,
	) -> anyhow::Result<ParserData> {
		let mut parser_data =
			self.parse_template_only(doc_params, template_name, layout, widget_id, template_parameters)?;
		// Collect components contained in this freshly-parsed template
		self.data.components.append(&mut parser_data.components);
		Ok(parser_data)
	}

	/// Parse named <template> tag and process it.
	/// Semi-internal - This function is suitable in cases if you don't want to pollute
	/// the main parser state state with dynamic IDs (this won't propagate components!)
	/// Use `realize_template` (or in some rare cases: `instantiate_template`) instead unless you want to handle `components` results yourself.
	/// Make sure not to drop resulting `ParserData` if you want to have your listener handles valid
	/// (they are contained in components). Use `realize_template` instead if you don't want to think about it.
	pub fn parse_template_only(
		&self,
		doc_params: &ParseDocumentParams,
		template_name: &str,
		layout: &mut Layout,
		widget_id: WidgetID,
		template_parameters: TemplateParams,
	) -> anyhow::Result<ParserData> {
		let Some(template) = self.data.templates.get(template_name) else {
			anyhow::bail!(
				"{:?}: no template named \"{template_name}\" found",
				self.xml_path.get_path().display()
			);
		};

		let mut ctx = ParserContext {
			layout,
			data_global: &self.data,
			data_local: ParserData::default(),
			doc_params,
		};

		let file = ParserFile {
			document: template.node_document.clone(),
			xml_path_source: self.xml_path_source,
			xml_path: self.xml_path.clone(),
			xml_dir: self.xml_dir.clone(),
			root_dir: self.root_dir.clone(),
			template_parameters: template_parameters.clone(), // FIXME: prevent copying
			version: self.version,
		};

		let _ = parse_widget_other_internal(&template.clone(), template_parameters, &file, &mut ctx, widget_id)?;
		Ok(ctx.data_local)
	}

	/// Parse named <template> tag and process it.
	/// Instantiate template by saving all the results into the main `ParserState`.
	/// Be aware you this function will save ALL parsed IDs and other metadata
	/// into your main `ParserState` context (deep move).
	/// You shouldn't instantiate the same template twice, to prevent ID name clash.
	/// Consider using `parse_template_only` or `realize_template` instead if you want
	/// to instantiate more than a single template of the same type.
	pub fn instantiate_template(
		&mut self,
		doc_params: &ParseDocumentParams,
		template_name: &str,
		layout: &mut Layout,
		widget_id: WidgetID,
		template_parameters: TemplateParams,
	) -> anyhow::Result<()> {
		let mut data_local = self.parse_template_only(doc_params, template_name, layout, widget_id, template_parameters)?;

		self.data.take_results_from(&mut data_local);
		Ok(())
	}

	pub(crate) fn context_menu_parse_cells(
		&mut self,
		template_name: &str,
		template_params: &TemplateParams,
	) -> anyhow::Result<Vec<context_menu::Cell>> {
		let Some(template) = self.data.templates.get(template_name) else {
			anyhow::bail!("no template named \"{template_name}\" found");
		};

		let doc = template.node_document.borrow_doc();
		let node = doc.get_node(template.node).context("node not found")?;
		let el_context_menu = node.first_element_child().context("child not found")?;
		let tag_name = el_context_menu.tag_name().name();
		if tag_name != "context_menu" {
			anyhow::bail!("expected <context_menu> tag, got <{tag_name}>");
		}

		let mut cells = Vec::<context_menu::Cell>::new();

		'children: for child in el_context_menu.children() {
			match child.tag_name().name() {
				"" => {}
				"cell" => {
					let mut title: Option<Translation> = None;
					let mut tooltip: Option<Translation> = None;
					let mut action_name: Option<Rc<str>> = None;
					let mut attribs = Vec::<AttribPair>::new();

					for attrib in child.attributes() {
						let (key, value) = (attrib.name(), attrib.value());

						match key {
							"text" => title = Some(Translation::from_raw_text(value)),
							"translation" => title = Some(Translation::from_translation_key(value)),
							"tooltip" => tooltip = Some(Translation::from_translation_key(value)),
							"tooltip_str" => tooltip = Some(Translation::from_raw_text(value)),
							"action" => action_name = Some(value.into()),
							"skip" => {
								let resolved = replace_vars(value, template_params);
								//FIXME: this is always empty
								if &*resolved == "1" {
									continue 'children;
								}
							}
							other => {
								if !other.starts_with('_') {
									anyhow::bail!("unexpected \"{other}\" attribute");
								}
								attribs.push(AttribPair::new(key, replace_vars(value, template_params)));
							}
						}
					}

					let title = title.context("No text/translation provided")?;
					cells.push(context_menu::Cell {
						title,
						tooltip,
						action_name,
						attribs,
					});
				}
				other => {
					anyhow::bail!("{:?}: unexpected <{other}> tag", self.xml_path.get_path().display());
				}
			}
		}

		Ok(cells)
	}
}

// convenience wrapper functions for `data`
impl Fetchable for ParserState {
	fn fetch_component_by_id(&self, id: &str) -> anyhow::Result<Component> {
		self.data.fetch_component_by_id(id)
	}

	fn fetch_component_by_widget_id(&self, state: &LayoutState, widget_id: WidgetID) -> anyhow::Result<Component> {
		self.data.fetch_component_by_widget_id(state, widget_id)
	}

	fn fetch_component_from_widget_id_as<T: 'static>(
		&self,
		state: &LayoutState,
		widget_id: WidgetID,
	) -> anyhow::Result<Rc<T>> {
		self.data.fetch_component_from_widget_id_as(state, widget_id)
	}

	fn fetch_component_as<T: 'static>(&self, id: &str) -> anyhow::Result<Rc<T>> {
		self.data.fetch_component_as(id)
	}

	fn get_widget_id(&self, id: &str) -> anyhow::Result<WidgetID> {
		self.data.get_widget_id(id)
	}

	fn fetch_widget(&self, state: &LayoutState, id: &str) -> anyhow::Result<WidgetPair> {
		self.data.fetch_widget(state, id)
	}

	fn fetch_widget_as<'a, T: 'static>(&self, state: &'a LayoutState, id: &str) -> anyhow::Result<RefMut<'a, T>> {
		self.data.fetch_widget_as(state, id)
	}
}

#[derive(Debug, Clone)]
struct MacroAttribs {
	attribs: HashMap<Rc<str>, Rc<str>>,
}

struct ParserContext<'a> {
	doc_params: &'a ParseDocumentParams<'a>,
	layout: &'a mut Layout,
	data_global: &'a ParserData, // current parser state at a given moment
	data_local: ParserData,      // newly processed items in a given template
}

impl ParserContext<'_> {
	const fn get_construct_essentials(&mut self, parent: WidgetID) -> ConstructEssentials<'_> {
		ConstructEssentials {
			layout: self.layout,
			parent,
		}
	}

	fn get_template(&self, name: &str) -> Option<Rc<Template>> {
		// find in local
		if let Some(template) = self.data_local.templates.get(name) {
			return Some(template.clone());
		}

		// find in global
		if let Some(template) = self.data_global.templates.get(name) {
			return Some(template.clone());
		}

		None
	}

	fn get_var(&self, name: &str) -> Option<Rc<str>> {
		// find in local
		if let Some(value) = self.data_local.var_map.get(name) {
			return Some(value.clone());
		}

		// find in global
		if let Some(value) = self.data_global.var_map.get(name) {
			return Some(value.clone());
		}

		None
	}

	fn get_macro_attrib(&self, value: &str) -> Option<&MacroAttribs> {
		// find in local
		if let Some(macro_attribs) = self.data_local.macro_attribs.get(value) {
			return Some(macro_attribs);
		}

		// find in global
		if let Some(macro_attribs) = self.data_global.macro_attribs.get(value) {
			return Some(macro_attribs);
		}

		None
	}

	fn insert_template(&mut self, name: Rc<str>, template: Rc<Template>) {
		self.data_local.templates.insert(name, template);
	}

	fn insert_var(&mut self, key: &str, value: &str) {
		self.data_local.var_map.insert(Rc::from(key), Rc::from(value));
	}

	fn insert_macro_attrib(&mut self, name: Rc<str>, attribs: MacroAttribs) {
		self.data_local.macro_attribs.insert(name, attribs);
	}

	fn insert_component(&mut self, widget_id: WidgetID, component: Component, id: Option<Rc<str>>) {
		self
			.layout
			.state
			.components_by_widget_id
			.insert(widget_id, component.weak());

		if let Some(id) = id
			&& self
				.data_local
				.components_by_id
				.insert(id.clone(), component.weak())
				.is_some()
		{
			log::warn!("{}: duplicate component ID \"{id}\"", self.doc_params.path.get_str());
		}

		self.data_local.components.push(component);
	}

	fn insert_id(&mut self, id: &Rc<str>, widget_id: WidgetID) {
		if self.data_local.ids.insert(id.clone(), widget_id).is_some() {
			log::warn!("{}: duplicate widget ID \"{id}\"", self.doc_params.path.get_str());
		}
	}

	fn populate_extra_variables(&mut self, other: &HashMap<Rc<str>, Rc<str>>) {
		for (k, v) in other {
			self.data_local.var_map.insert(k.clone(), v.clone());
		}
	}

	fn print_invalid_attrib(&self, tag_name: &str, key: &str, value: &str) {
		log::warn!(
			"{}: <{tag_name}> value for \"{key}\" is invalid: \"{value}\"",
			self.doc_params.path.get_str()
		);
	}

	fn print_invalid_tag(&self, tag_name: &str, invalid_tag_name: &str) {
		log::warn!(
			"{}: <{tag_name}> has an invalid tag named <{invalid_tag_name}>",
			self.doc_params.path.get_str()
		);
	}

	fn print_missing_attrib(&self, tag_name: &str, attr: &str) {
		log::warn!(
			"{}: <{tag_name}> is missing \"{attr}\".",
			self.doc_params.path.get_str()
		);
	}

	fn parse_val(&self, tag_name: &str, key: &str, value: &str) -> Option<f32> {
		let Ok(val) = value.parse::<f32>() else {
			self.print_invalid_attrib(tag_name, key, value);
			return None;
		};
		Some(val)
	}

	fn parse_percent(&self, tag_name: &str, key: &str, value: &str) -> Option<f32> {
		let Some(val_str) = value.split('%').next() else {
			self.print_invalid_attrib(tag_name, key, value);
			return None;
		};

		let Ok(val) = val_str.parse::<f32>() else {
			self.print_invalid_attrib(tag_name, key, value);
			return None;
		};
		Some(val / 100.0)
	}

	pub fn parse_auto(value: &str) -> bool {
		value.contains("auto")
	}

	fn parse_size_unit<T>(&self, tag_name: &str, key: &str, value: &str) -> Option<T>
	where
		T: taffy::prelude::FromPercent + taffy::prelude::FromLength,
	{
		if is_percent(value) {
			Some(taffy::prelude::percent(self.parse_percent(tag_name, key, value)?))
		} else {
			Some(taffy::prelude::length(parse_f32(value)?))
		}
	}

	fn parse_check_i32(&self, tag_name: &str, key: &str, value: &str, num: &mut i32) -> bool {
		if let Some(value) = parse_i32(value) {
			*num = value;
			true
		} else {
			self.print_invalid_attrib(tag_name, key, value);
			false
		}
	}

	fn parse_check_f32(&self, tag_name: &str, key: &str, value: &str, num: &mut f32) -> bool {
		if let Some(value) = parse_f32(value) {
			*num = value;
			true
		} else {
			self.print_invalid_attrib(tag_name, key, value);
			false
		}
	}
}

fn parse_i32(value: &str) -> Option<i32> {
	value.parse::<i32>().ok()
}

fn parse_f32(value: &str) -> Option<f32> {
	value.parse::<f32>().ok()
}

fn is_percent(value: &str) -> bool {
	value.ends_with('%')
}

fn get_tag_by_name<'a>(node: &roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
	node.children().find(|&child| child.tag_name().name() == name)
}

fn require_tag_by_name<'a>(node: &roxmltree::Node<'a, 'a>, name: &str) -> anyhow::Result<roxmltree::Node<'a, 'a>> {
	get_tag_by_name(node, name).ok_or_else(|| anyhow::anyhow!("Tag \"{name}\" not found"))
}

fn parse_widget_other_internal(
	template: &Rc<Template>,
	template_parameters: TemplateParams,
	file: &ParserFile,
	ctx: &mut ParserContext,
	parent_id: WidgetID,
) -> anyhow::Result<ParseChildResult> {
	let template_file = ParserFile {
		document: template.node_document.clone(),
		version: file.version,
		template_parameters,
		root_dir: template.root_dir.clone(),
		xml_path_source: template.xml_path_source,
		xml_path: template.xml_path.clone(),
		xml_dir: template.xml_dir.clone(),
	};

	let doc = template_file.document.clone();

	let template_node = doc
		.borrow_doc()
		.get_node(template.node)
		.context("template node invalid")?;

	parse_children(&template_file, ctx, template_node, parent_id)
}

fn parse_widget_other<'a>(
	xml_tag_name: &str,
	file: &ParserFile,
	ctx: &mut ParserContext,
	node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
	attribs: &[AttribPair],
) -> anyhow::Result<()> {
	let Some(template) = ctx.get_template(xml_tag_name) else {
		log::error!(
			"{}: Undefined tag named \"{xml_tag_name}\"",
			ctx.doc_params.path.get_str()
		);
		return Ok(()); // not critical
	};

	let template_params: HashMap<Rc<str>, Rc<str>> =
		attribs.iter().map(|a| (a.attrib.clone(), a.value.clone())).collect();

	// parse template body
	let res = parse_widget_other_internal(
		&template,
		TemplateParams::from_hashmap(template_params),
		file,
		ctx,
		parent_id,
	)?;

	if let Some(children_parent_id) = res.template_children_parent_id {
		let _ = parse_children(file, ctx, node, children_parent_id)?;
	}

	Ok(())
}

fn strip_starting_slash(input: &str) -> &str {
	if let Some(c) = input.chars().next()
		&& c == '/'
	{
		return &input[1..];
	}
	input
}

pub struct ExpandPathParams<'a> {
	pub version: Version,
	pub root_dir: &'a AssetPathRc,
	pub xml_dir: &'a AssetPathRc,
}

impl<'a> ExpandPathParams<'a> {
	pub const fn from_parser_state(state: &'a ParserState) -> ExpandPathParams<'a> {
		ExpandPathParams {
			version: state.version,
			root_dir: &state.root_dir,
			xml_dir: &state.xml_dir,
		}
	}
}

pub fn expand_path(par: &ExpandPathParams, path_source: AssetPathSource, path_string: &str) -> PathBuf {
	if par.version.0 <= 1 {
		return normalize_path(Path::new(path_string), false);
	}

	if let Some(c) = path_string.chars().next()
		&& c == '@'
	{
		let path_without_at = &path_string[1..];
		let rel_path = strip_starting_slash(path_without_at);

		let joined = match path_source {
			AssetPathSource::Internal | AssetPathSource::BuiltIn => PathBuf::from(rel_path),
			AssetPathSource::Filesystem => par.root_dir.get_path().join(rel_path),
		};

		return normalize_path(&joined, false);
	}

	let relative_dir = par.xml_dir.get_path();
	normalize_path(&relative_dir.join(path_string), false)
}

// attrib needs to be "src_internal", "src_builtin", "src_ext" or "src"
fn expand_path_from_kv(file: &ParserFile, attrib: &str, value: &str) -> AssetPathRc {
	let par = ExpandPathParams {
		version: file.version,
		root_dir: &file.root_dir,
		xml_dir: &file.xml_dir,
	};

	match attrib {
		"src_internal" => AssetPathRc::WguiInternal(expand_path(&par, AssetPathSource::Internal, value).into()),
		"src_builtin" => AssetPathRc::BuiltIn(expand_path(&par, AssetPathSource::BuiltIn, value).into()),
		"src" => AssetPathRc::FileOrBuiltIn(
			expand_path(
				&par,
				if file.xml_path_source == AssetPathSource::Filesystem {
					AssetPathSource::Filesystem // use filesystem for src="..." if the xml file resides on the filesystem too
				} else {
					AssetPathSource::BuiltIn // use builtin
				},
				value,
			)
			.into(),
		),
		"src_ext" => AssetPathRc::File(expand_path(&par, AssetPathSource::Filesystem, value).into()),
		_ => unreachable!(),
	}
}

fn parse_tag_include(
	file: &ParserFile,
	ctx: &mut ParserContext,
	parent_id: WidgetID,
	attribs: &[AttribPair],
) -> anyhow::Result<()> {
	const TAG_NAME: &str = "include";

	let mut path = None;
	let mut optional = false;

	for pair in attribs {
		#[allow(clippy::single_match)]
		match pair.attrib.as_ref() {
			"src" | "src_ext" | "src_builtin" | "src_internal" => {
				path = Some(expand_path_from_kv(file, &pair.attrib, &pair.value));
			}
			"optional" => {
				let mut optional_i32 = 0;
				optional = ctx.parse_check_i32(TAG_NAME, &pair.attrib, &pair.value, &mut optional_i32) && optional_i32 == 1;
			}
			_ => {
				ctx.print_invalid_attrib(TAG_NAME, pair.attrib.as_ref(), pair.value.as_ref());
			}
		}
	}

	let Some(path) = path else {
		ctx.print_missing_attrib("include", "src");
		return Ok(());
	};
	let path_ref = path.as_ref();
	match get_doc_from_xml_asset_path(ctx, &file.root_dir, path_ref) {
		Ok((new_file, node_layout)) => parse_document_root(&new_file, ctx, parent_id, node_layout)?,
		Err(e) => {
			if !optional {
				return Err(e);
			}
		}
	}

	Ok(())
}

fn parse_tag_var<'a>(ctx: &mut ParserContext, tag_name: &str, node: roxmltree::Node<'a, 'a>) {
	let mut out_key: Option<&str> = None;
	let mut out_value: Option<&str> = None;

	for attrib in node.attributes() {
		let (key, value) = (attrib.name(), attrib.value());

		match key {
			"key" => {
				out_key = Some(value);
			}
			"value" => {
				out_value = Some(value);
			}
			_ => {
				ctx.print_invalid_attrib(tag_name, key, value);
			}
		}
	}

	let Some(key) = out_key else {
		ctx.print_missing_attrib(tag_name, "key");
		return;
	};

	let Some(value) = out_value else {
		ctx.print_missing_attrib(tag_name, "value");
		return;
	};

	ctx.insert_var(key, value);
}

pub fn replace_vars(input: &str, vars: &TemplateParams) -> Rc<str> {
	let re = regex::Regex::new(r"\$\{([^}]*)\}").unwrap();

	/*if !vars.is_empty() {
		log::error!("template parameters {:?}", vars);
	}*/

	let out = re.replace_all(input, |captures: &regex::Captures| {
		let input_var = &captures[1];

		if let Some(replacement) = vars.0.get(input_var) {
			replacement.clone()
		} else {
			// failed to find var, return an empty string
			Rc::from("")
		}
	});

	Rc::from(out)
}

#[allow(clippy::manual_strip)]
#[allow(clippy::single_match_else)]
fn process_attrib_internal(
	template_parameters: &TemplateParams,
	ctx: &ParserContext,
	key: &str,
	value: &str,
) -> AttribPair {
	if value.starts_with('~') {
		let name = &value[1..];

		match ctx.get_var(name) {
			Some(name) => AttribPair::new(key, name),
			None => {
				log::warn!("{}: undefined variable \"{value}\"", ctx.doc_params.path.get_str());
				AttribPair::new(key, format!("undefined_{value}"))
			}
		}
	} else {
		AttribPair::new(key, replace_vars(value, template_parameters))
	}
}

fn process_attrib(
	template_parameters: &TemplateParams,
	ctx: &ParserContext,
	key: &str,
	value: &str,
) -> Option<AttribPair> {
	let pair = process_attrib_internal(template_parameters, ctx, key, value);
	if pair.value.is_empty() {
		return None;
	}
	Some(pair)
}

fn raw_attribs<'a>(node: &'a roxmltree::Node<'a, 'a>) -> Vec<AttribPair> {
	let mut res = vec![];
	for attrib in node.attributes() {
		let (key, value) = (attrib.name(), attrib.value());
		res.push(AttribPair::new(key, value));
	}
	res
}

fn process_attribs<'a>(
	file: &'a ParserFile,
	ctx: &'a ParserContext,
	node: &'a roxmltree::Node<'a, 'a>,
	is_tag_macro: bool,
) -> Vec<AttribPair> {
	if is_tag_macro {
		// return as-is, no attrib post-processing
		return raw_attribs(node);
	}
	let mut res = vec![];

	for attrib in node.attributes() {
		let (key, value) = (attrib.name(), attrib.value());

		if key == "macro" {
			if let Some(macro_attrib) = ctx.get_macro_attrib(value) {
				for (macro_key, macro_value) in &macro_attrib.attribs {
					if let Some(pair) = process_attrib(&file.template_parameters, ctx, macro_key, macro_value) {
						res.push(pair);
					}
				}
			} else {
				log::warn!(
					"{}: requested macro named \"{value}\" not found!",
					ctx.doc_params.path.get_str()
				);
			}
		} else {
			if let Some(pair) = process_attrib(&file.template_parameters, ctx, key, value) {
				res.push(pair);
			}
		}
	}

	res
}

fn parse_tag_vars<'a>(ctx: &mut ParserContext, node: roxmltree::Node<'a, 'a>) {
	for child_node in node.children() {
		let child_name = child_node.tag_name().name();
		match child_name {
			"var" => {
				parse_tag_var(ctx, child_name, child_node);
			}
			"" => { /* ignore */ }
			_ => {
				log::warn!(
					"{}: <{child_name}> is not a valid child to <vars>.",
					ctx.doc_params.path.get_str()
				);
			}
		}
	}
}

fn parse_tag_template(file: &ParserFile, ctx: &mut ParserContext, node: roxmltree::Node<'_, '_>) {
	let mut template_name: Option<Rc<str>> = None;

	let attribs = process_attribs(file, ctx, &node, false);

	for pair in attribs {
		match pair.attrib.as_ref() {
			"name" => {
				template_name = Some(pair.value);
			}
			_ => {
				ctx.print_invalid_attrib("template", &pair.attrib, pair.value.as_ref());
			}
		}
	}

	let Some(name) = template_name else {
		ctx.print_missing_attrib("template", "name");
		return;
	};

	ctx.insert_template(
		name,
		Rc::new(Template {
			node: node.id(),
			node_document: file.document.clone(),
			root_dir: file.root_dir.clone(),
			xml_path_source: file.xml_path_source,
			xml_path: file.xml_path.clone(),
			xml_dir: file.xml_dir.clone(),
		}),
	);
}

fn parse_tag_macro(file: &ParserFile, ctx: &mut ParserContext, node: roxmltree::Node<'_, '_>) {
	let mut macro_name: Option<Rc<str>> = None;

	let attribs = process_attribs(file, ctx, &node, true);
	let mut macro_attribs = HashMap::<Rc<str>, Rc<str>>::new();

	for pair in attribs {
		match pair.attrib.as_ref() {
			"name" => {
				macro_name = Some(pair.value);
			}
			_ => {
				if macro_attribs.insert(pair.attrib.clone(), pair.value).is_some() {
					log::warn!(
						"{}: macro attrib \"{}\" already defined!",
						ctx.doc_params.path.get_str(),
						pair.attrib
					);
				}
			}
		}
	}

	let Some(name) = macro_name else {
		ctx.print_missing_attrib("macro", "name");
		return;
	};

	ctx.insert_macro_attrib(name, MacroAttribs { attribs: macro_attribs });
}

fn process_component(ctx: &mut ParserContext, component: Component, widget_id: WidgetID, attribs: &[AttribPair]) {
	let mut component_id: Option<Rc<str>> = None;

	for pair in attribs {
		#[allow(clippy::single_match)]
		match pair.attrib.as_ref() {
			"id" => {
				component_id = Some(pair.value.clone());
			}
			_ => {}
		}
	}

	ctx.insert_component(widget_id, component, component_id);
}

fn parse_widget_universal(ctx: &mut ParserContext, widget: &WidgetPair, attribs: &[AttribPair], tag_name: &str) {
	for pair in attribs {
		#[allow(clippy::single_match)]
		match pair.attrib.as_ref() {
			"id" => {
				// Attach a specific widget to name-ID map (just like getElementById)
				ctx.insert_id(&pair.value, widget.id);
			}
			"new_pass" => {
				if let Some(num) = parse_i32(&pair.value) {
					widget.widget.state().flags.new_pass = num != 0;
				} else {
					ctx.print_invalid_attrib(tag_name, &pair.attrib, &pair.value);
				}
			}
			"interactable" => {
				if let Some(num) = parse_i32(&pair.value) {
					widget.widget.state().flags.interactable = num != 0;
				} else {
					ctx.print_invalid_attrib(tag_name, &pair.attrib, &pair.value);
				}
			}
			"consume_mouse_events" => {
				if let Some(num) = parse_i32(&pair.value) {
					widget.widget.state().flags.consume_mouse_events = num != 0;
				} else {
					ctx.print_invalid_attrib(tag_name, &pair.attrib, &pair.value);
				}
			}
			_ => {}
		}
	}
}

fn parse_child<'a>(
	file: &ParserFile,
	ctx: &mut ParserContext,
	child_node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
) -> anyhow::Result<ParseChildResult> {
	let tag_name = child_node.tag_name().name();
	if let Some(skip) = child_node.attribute("skip")
		&& let Some(pair) = process_attrib(&file.template_parameters, ctx, "skip", skip)
	{
		let resolved = pair.value;
		if &*resolved == "1" {
			return Ok(ParseChildResult::default()); // do not parse this element
		}
	}

	let attribs = process_attribs(file, ctx, &child_node, false);

	let (res, new_widget_id) = match tag_name {
		"include" => {
			parse_tag_include(file, ctx, parent_id, &attribs)?;
			(ParseChildResult::default(), None)
		}
		"div" => {
			let (res, id) = parse_widget_div(file, ctx, child_node, parent_id, &attribs, tag_name)?;
			(res, Some(id))
		}
		"rectangle" => {
			let (res, id) = parse_widget_rectangle(file, ctx, child_node, parent_id, &attribs, tag_name)?;
			(res, Some(id))
		}
		"label" => {
			let (res, id) = parse_widget_label(file, ctx, child_node, parent_id, &attribs, tag_name)?;
			(res, Some(id))
		}
		"sprite" => {
			let (res, id) = parse_widget_sprite(file, ctx, child_node, parent_id, &attribs, tag_name)?;
			(res, Some(id))
		}
		"image" => {
			let (res, id) = parse_widget_image(file, ctx, child_node, parent_id, &attribs, tag_name)?;
			(res, Some(id))
		}
		"Button" => {
			let (res, id) = parse_component_button(file, ctx, child_node, parent_id, &attribs, tag_name)?;
			(res, Some(id))
		}
		#[cfg(feature = "video")]
		"Video" => {
			use crate::parser::component_video::parse_component_video;
			let (res, id) = parse_component_video(file, ctx, child_node, parent_id, &attribs, tag_name)?;
			(res, Some(id))
		}
		"Slider" => (
			Default::default(),
			Some(parse_component_slider(ctx, parent_id, &attribs, tag_name)?),
		),
		"ColorSelector" => (
			Default::default(),
			Some(parse_component_color_selector(ctx, parent_id, &attribs, tag_name)?),
		),
		"CheckBox" => (
			Default::default(),
			Some(parse_component_checkbox(
				ctx,
				parent_id,
				&attribs,
				tag_name,
				CheckboxKind::CheckBox,
			)?),
		),
		"RadioBox" => (
			Default::default(),
			Some(parse_component_checkbox(
				ctx,
				parent_id,
				&attribs,
				tag_name,
				CheckboxKind::RadioBox,
			)?),
		),
		"RadioGroup" => (
			Default::default(),
			Some(parse_component_radio_group(
				file, ctx, child_node, parent_id, &attribs, tag_name,
			)?),
		),
		"EditBox" => (
			Default::default(),
			Some(parse_component_editbox(ctx, parent_id, &attribs, tag_name)?),
		),
		"BarGraph" => (
			Default::default(),
			Some(parse_component_bar_graph(ctx, parent_id, &attribs, tag_name)?),
		),
		"Tabs" => (
			Default::default(),
			Some(parse_component_tabs(
				file, ctx, child_node, parent_id, &attribs, tag_name,
			)?),
		),
		"CHILDREN" => (
			ParseChildResult {
				template_children_parent_id: Some(parent_id),
			},
			None,
		),
		"" => {
			(Default::default(), None) /* ignore */
		}
		other_tag_name => {
			parse_widget_other(other_tag_name, file, ctx, child_node, parent_id, &attribs)?;
			(Default::default(), None)
		}
	};

	// check for custom attributes (if the callback is set)
	if let Some(widget_id) = new_widget_id
		&& let Some(on_custom_attribs) = &ctx.doc_params.extra.on_custom_attribs
	{
		let mut pairs = SmallVec::<[AttribPair; 4]>::new();

		for pair in attribs {
			if !pair.attrib.starts_with('_') || pair.attrib.is_empty() {
				continue;
			}
			pairs.push(pair.clone());
		}

		if !pairs.is_empty() {
			on_custom_attribs(CustomAttribsInfo {
				widgets: &ctx.layout.state.widgets,
				parent_id,
				widget_id,
				pairs: &pairs,
			});
		}
	}

	Ok(res)
}

#[must_use]
#[derive(Default)]
struct ParseChildResult {
	// parent widget id of <CHILDREN/> tag
	// available only if we're parsing a template
	template_children_parent_id: Option<WidgetID>,
}

impl ParseChildResult {
	#[allow(clippy::needless_pass_by_value)]
	fn consume(&mut self, res: ParseChildResult) {
		if let Some(id) = res.template_children_parent_id {
			if self.template_children_parent_id.is_some() {
				log::warn!("Found more than a single <CHILDREN/> instance in a template");
			}

			self.template_children_parent_id = Some(id);
		}
	}
}

fn parse_children<'a>(
	file: &ParserFile,
	ctx: &mut ParserContext,
	parent_node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
) -> anyhow::Result<ParseChildResult> {
	let mut res = ParseChildResult::default();

	for child_node in parent_node.children() {
		res.consume(parse_child(file, ctx, child_node, parent_id)?);
	}

	Ok(res)
}

fn create_default_context<'a>(
	doc_params: &'a ParseDocumentParams,
	layout: &'a mut Layout,
	data_global: &'a ParserData,
) -> ParserContext<'a> {
	ParserContext {
		doc_params,
		layout,
		data_local: ParserData::default(),
		data_global,
	}
}

#[derive(Debug, Clone)]
pub struct AttribPair {
	pub attrib: Rc<str>,
	pub value: Rc<str>,
}

impl AttribPair {
	fn new<A, V>(attrib: A, value: V) -> Self
	where
		A: Into<Rc<str>>,
		V: Into<Rc<str>>,
	{
		Self {
			attrib: attrib.into(),
			value: value.into(),
		}
	}
}

pub struct CustomAttribsInfo<'a> {
	pub parent_id: WidgetID,
	pub widget_id: WidgetID,
	pub widgets: &'a WidgetMap,
	pub pairs: &'a [AttribPair],
}

// helper functions
impl CustomAttribsInfo<'_> {
	pub fn get_widget(&self) -> Option<&Widget> {
		self.widgets.get(self.widget_id)
	}

	pub fn get_widget_as<T: 'static>(&self) -> Option<RefMut<'_, T>> {
		self.widgets.get(self.widget_id)?.get_as::<T>()
	}

	pub fn get_value(&self, attrib_name: &str) -> Option<Rc<str>> {
		// O(n) search, these pairs won't be problematically big anyways
		for pair in self.pairs {
			if *pair.attrib == *attrib_name {
				return Some(pair.value.clone());
			}
		}

		None
	}

	pub fn to_owned(&self) -> CustomAttribsInfoOwned {
		CustomAttribsInfoOwned {
			parent_id: self.parent_id,
			widget_id: self.widget_id,
			pairs: self.pairs.to_vec(),
		}
	}
}

pub struct CustomAttribsInfoOwned {
	pub parent_id: WidgetID,
	pub widget_id: WidgetID,
	pub pairs: Vec<AttribPair>,
}

impl CustomAttribsInfoOwned {
	pub fn get_value(&self, attrib_name: &str) -> Option<&str> {
		// O(n) search, these pairs won't be problematically big anyways
		for pair in &self.pairs {
			if pair.attrib.as_ref() == attrib_name {
				return Some(pair.value.as_ref());
			}
		}

		None
	}
}

pub type OnCustomAttribsFunc = Rc<dyn Fn(CustomAttribsInfo)>;

#[derive(Default, Clone)]
pub struct ParseDocumentExtra {
	pub on_custom_attribs: Option<OnCustomAttribsFunc>, // all attributes with '_' character prepended
	pub dev_mode: bool,
	pub extra_vars: HashMap<Rc<str>, Rc<str>>,
	pub root_dir: Option<AssetPathRc>,
}

// filled-in by you in `new_layout_from_assets` function
pub struct ParseDocumentParams<'a> {
	pub globals: WguiGlobals,      // mandatory field
	pub path: AssetPathRef<'a>,    // XML path, mandatory field
	pub extra: ParseDocumentExtra, // optional field, can be Default-ed
}

pub fn parse_from_assets(
	doc_params: &ParseDocumentParams,
	layout: &mut Layout,
	parent_id: WidgetID,
) -> anyhow::Result<ParserState> {
	let parser_data = ParserData::default();
	let mut ctx = create_default_context(doc_params, layout, &parser_data);
	ctx.populate_extra_variables(&doc_params.extra.extra_vars);

	let xml_path = doc_params.path.to_rc();
	let xml_dir = xml_path.strip_filename();
	let root_dir = if let Some(root_dir) = &doc_params.extra.root_dir {
		root_dir.clone()
	} else {
		xml_path.replace_path(Path::new("/").into())
	};

	let (file, node_layout) = get_doc_from_xml_asset_path(&ctx, &root_dir, doc_params.path)?;
	parse_document_root(&file, &mut ctx, parent_id, node_layout)?;

	// move everything essential to the result
	let result = ParserState {
		data: std::mem::take(&mut ctx.data_local),
		xml_path_source: file.xml_path_source,
		xml_path,
		xml_dir,
		root_dir,
		version: file.version,
	};

	drop(ctx);

	Ok(result)
}

pub fn new_layout_from_assets(
	doc_params: &ParseDocumentParams,
	layout_params: LayoutParams,
) -> anyhow::Result<(Layout, ParserState)> {
	let mut layout = Layout::new(doc_params.globals.clone(), layout_params)?;
	let widget = layout.content_root_widget;
	let state = parse_from_assets(doc_params, &mut layout, widget)?;
	Ok((layout, state))
}

fn get_doc_from_xml_asset_path(
	ctx: &ParserContext,
	root_dir: &AssetPathRc,
	xml_asset_path: AssetPathRef,
) -> anyhow::Result<(ParserFile, roxmltree::NodeId)> {
	let (data, xml_path_source) = ctx.layout.state.globals.get_asset(xml_asset_path)?;
	let xml = String::from_utf8(data)?;

	let document = Rc::new(XmlDocument::new(xml, |xml| {
		let opt = roxmltree::ParsingOptions {
			allow_dtd: true,
			..Default::default()
		};
		roxmltree::Document::parse_with_options(xml, opt)
			.context("Unable to parse XML")
			.log_err_with(&xml_asset_path)
			.unwrap()
	}));

	let root = document.borrow_doc().root();
	let tag_layout = require_tag_by_name(&root, "layout")?;

	let xml_path = xml_asset_path.to_rc();
	let xml_dir = xml_path.strip_filename();

	#[allow(clippy::useless_let_if_seq)]
	let mut version = 1;

	if let Some(str_version) = tag_layout.attribute("version") {
		version = str_version.parse::<u32>()?;
	}

	if version == 0 || version > 2 {
		anyhow::bail!("unsupported layout version {version}");
	}

	if version == 1 {
		log::warn!(
			"<layout> without version specified, assuming it's version 1. Update your code by specifying <layout version=\"2\">."
		);
	}

	let file = ParserFile {
		document: document.clone(),
		xml_path_source,
		template_parameters: TemplateParams::new(),
		root_dir: root_dir.clone(),
		xml_path,
		xml_dir,
		version: Version(version),
	};

	Ok((file, tag_layout.id()))
}

fn parse_document_root(
	file: &ParserFile,
	ctx: &mut ParserContext,
	parent_id: WidgetID,
	node_layout: roxmltree::NodeId,
) -> anyhow::Result<()> {
	let node_layout = file
		.document
		.borrow_doc()
		.get_node(node_layout)
		.context("layout node not found")?;

	for child_node in node_layout.children() {
		match child_node.tag_name().name() {
			/*  topmost include directly in <layout>  */
			"include" => parse_tag_include(file, ctx, parent_id, &raw_attribs(&child_node))?,
			"vars" => parse_tag_vars(ctx, child_node),
			"theme" => {
				log::error!("Using deprecated <theme> tag. Use <vars> instead.");
				parse_tag_vars(ctx, child_node);
			}
			"template" => parse_tag_template(file, ctx, child_node),
			"blueprint" => parse_tag_template(file, ctx, child_node),
			"macro" => parse_tag_macro(file, ctx, child_node),
			_ => {}
		}
	}

	if let Some(tag_elements) = get_tag_by_name(&node_layout, "elements") {
		let _ = parse_children(file, ctx, tag_elements, parent_id)?;
	}

	Ok(())
}

fn get_asset_path_from_kv<'a>(file: &ParserFile, prefix: &str, key: &'a str, value: &'a str) -> AssetPathRc {
	let key = key.strip_prefix(prefix).unwrap_or(key);
	expand_path_from_kv(file, key, value)
}
