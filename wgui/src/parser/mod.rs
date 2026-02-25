mod component_bar_graph;
mod component_button;
mod component_checkbox;
mod component_editbox;
mod component_radio_group;
mod component_slider;
mod component_tabs;
mod helpers;
mod style;
mod widget_div;
mod widget_image;
mod widget_label;
mod widget_rectangle;
mod widget_sprite;

use crate::{
	assets::{AssetPath, AssetPathOwned, normalize_path},
	components::{Component, ComponentWeak},
	drawing::{self},
	globals::WguiGlobals,
	i18n::Translation,
	layout::{Layout, LayoutParams, LayoutState, Widget, WidgetID, WidgetMap, WidgetPair},
	log::LogErr,
	parser::{
		component_bar_graph::parse_component_bar_graph,
		component_button::parse_component_button,
		component_checkbox::{CheckboxKind, parse_component_checkbox},
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
use std::{cell::RefMut, collections::HashMap, path::Path, rc::Rc};

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
}

struct ParserFile {
	path: AssetPathOwned,
	document: Rc<XmlDocument>,
	template_parameters: HashMap<Rc<str>, Rc<str>>,
}

/*
	`components` could contain connected listener handles.
		Do not drop them unless you don't need to handle any events,
		including mouse-hover animations.
*/
#[derive(Default, Clone)]
pub struct ParserData {
	pub components_by_id: HashMap<Rc<str>, ComponentWeak>,
	pub components_by_widget_id: HashMap<WidgetID, ComponentWeak>,
	pub components: Vec<Component>,
	pub ids: HashMap<Rc<str>, WidgetID>,
	pub templates: HashMap<Rc<str>, Rc<Template>>,
	pub var_map: HashMap<Rc<str>, Rc<str>>,
	macro_attribs: HashMap<Rc<str>, MacroAttribs>,
}

pub trait Fetchable {
	/// Return a component by its string ID
	fn fetch_component_by_id(&self, id: &str) -> anyhow::Result<Component>;

	/// Return a component by the ID of the widget that owns it
	fn fetch_component_by_widget_id(&self, widget_id: WidgetID) -> anyhow::Result<Component>;

	/// Fetch a component by string ID and down‑cast it to a concrete component type `T` (see `components/mod.rs`)
	fn fetch_component_as<T: 'static>(&self, id: &str) -> anyhow::Result<Rc<T>>;

	/// Fetch a component by widget ID and down‑cast it to a concrete component type `T` (see `components/mod.rs`)
	fn fetch_component_from_widget_id_as<T: 'static>(&self, widget_id: WidgetID) -> anyhow::Result<Rc<T>>;

	/// Return a widget by its string ID
	fn get_widget_id(&self, id: &str) -> anyhow::Result<WidgetID>;

	/// Retrieve the widget associated with a string ID, returning a `WidgetPair` (id and widget itself)
	fn fetch_widget(&self, state: &LayoutState, id: &str) -> anyhow::Result<WidgetPair>;

	/// Retrieve a widget by string ID and down‑cast its inner value to type `T` (see `widget/mod.rs`)
	fn fetch_widget_as<'a, T: 'static>(&self, state: &'a LayoutState, id: &str) -> anyhow::Result<RefMut<'a, T>>;
}

impl ParserData {
	pub(crate) fn take_results_from(&mut self, from: &mut Self) {
		let ids = std::mem::take(&mut from.ids);
		let components = std::mem::take(&mut from.components);
		let components_by_id = std::mem::take(&mut from.components_by_id);
		let components_by_widget_id = std::mem::take(&mut from.components_by_widget_id);

		for (id, key) in ids {
			self.ids.insert(id, key);
		}

		for c in components {
			self.components.push(c);
		}

		for (k, v) in components_by_id {
			self.components_by_id.insert(k, v);
		}

		for (k, v) in components_by_widget_id {
			self.components_by_widget_id.insert(k, v);
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

	fn fetch_component_by_widget_id(&self, widget_id: WidgetID) -> anyhow::Result<Component> {
		let Some(weak) = self.components_by_widget_id.get(&widget_id) else {
			anyhow::bail!("Component by widget ID \"{widget_id:?}\" doesn't exist");
		};

		let Some(component) = weak.upgrade() else {
			anyhow::bail!("Component by widget ID \"{widget_id:?}\" has disappeared");
		};

		Ok(Component(component))
	}

	fn fetch_component_as<T: 'static>(&self, id: &str) -> anyhow::Result<Rc<T>> {
		let component = self.fetch_component_by_id(id)?;

		if !(*component.0).as_any().is::<T>() {
			anyhow::bail!("fetch_component_as({id}): type not matching");
		}

		// safety: we just checked the type
		unsafe { Ok(Rc::from_raw(Rc::into_raw(component.0).cast())) }
	}

	fn fetch_component_from_widget_id_as<T: 'static>(&self, widget_id: WidgetID) -> anyhow::Result<Rc<T>> {
		let component = self.fetch_component_by_widget_id(widget_id)?;

		if !(*component.0).as_any().is::<T>() {
			anyhow::bail!("fetch_component_by_widget_id({widget_id:?}): type not matching");
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
#[derive(Default)]
pub struct ParserState {
	pub data: ParserData,
	pub path: AssetPathOwned,
}

impl ParserState {
	/// This function is suitable in cases if you don't want to pollute main parser state with dynamic IDs
	/// Use `instantiate_template` instead unless you want to handle `components` results yourself.
	/// Make sure not to drop them if you want to have your listener handles valid
	pub fn parse_template(
		&mut self,
		doc_params: &ParseDocumentParams,
		template_name: &str,
		layout: &mut Layout,
		widget_id: WidgetID,
		template_parameters: HashMap<Rc<str>, Rc<str>>,
	) -> anyhow::Result<ParserData> {
		let Some(template) = self.data.templates.get(template_name) else {
			anyhow::bail!(
				"{:?}: no template named \"{template_name}\" found",
				self.path.get_path_buf().display()
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
			path: self.path.clone(),
			template_parameters: template_parameters.clone(), // FIXME: prevent copying
		};

		parse_widget_other_internal(&template.clone(), template_parameters, &file, &mut ctx, widget_id)?;
		Ok(ctx.data_local)
	}

	/// Instantinate template by saving all the results into the main `ParserState`
	pub fn instantiate_template(
		&mut self,
		doc_params: &ParseDocumentParams,
		template_name: &str,
		layout: &mut Layout,
		widget_id: WidgetID,
		template_parameters: HashMap<Rc<str>, Rc<str>>,
	) -> anyhow::Result<()> {
		let mut data_local = self.parse_template(doc_params, template_name, layout, widget_id, template_parameters)?;

		self.data.take_results_from(&mut data_local);
		Ok(())
	}

	pub(crate) fn context_menu_parse_cells(
		&mut self,
		template_name: &str,
		template_params: &HashMap<Rc<str>, Rc<str>>,
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

		for child in el_context_menu.children() {
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
					anyhow::bail!("{:?}: unexpected <{other}> tag", self.path.get_path_buf().display());
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

	fn fetch_component_by_widget_id(&self, widget_id: WidgetID) -> anyhow::Result<Component> {
		self.data.fetch_component_by_widget_id(widget_id)
	}

	fn fetch_component_as<T: 'static>(&self, id: &str) -> anyhow::Result<Rc<T>> {
		self.data.fetch_component_as(id)
	}

	fn fetch_component_from_widget_id_as<T: 'static>(&self, widget_id: WidgetID) -> anyhow::Result<Rc<T>> {
		self.data.fetch_component_from_widget_id_as(widget_id)
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
			.data_local
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

	fn populate_theme_variables(&mut self) {
		let theme = self.layout.state.theme.clone();

		macro_rules! insert_color_vars {
			($self:expr, $name:literal, $field:expr, $alpha:expr) => {
				$self.insert_var(concat!("color_", $name), &$field.to_hex());
				$self.insert_var(
					concat!("color_", $name, "_translucent"),
					&$field.with_alpha($alpha).to_hex(),
				);
				$self.insert_var(concat!("color_", $name, "_50"), &$field.mult_rgb(0.50).to_hex());
				$self.insert_var(concat!("color_", $name, "_40"), &$field.mult_rgb(0.40).to_hex());
				$self.insert_var(concat!("color_", $name, "_30"), &$field.mult_rgb(0.30).to_hex());
				$self.insert_var(concat!("color_", $name, "_20"), &$field.mult_rgb(0.20).to_hex());
				$self.insert_var(concat!("color_", $name, "_10"), &$field.mult_rgb(0.10).to_hex());
			};
		}

		insert_color_vars!(self, "text", theme.text_color, theme.translucent_alpha);
		insert_color_vars!(self, "accent", theme.accent_color, theme.translucent_alpha);
		insert_color_vars!(self, "danger", theme.danger_color, theme.translucent_alpha);
		insert_color_vars!(self, "faded", theme.faded_color, theme.translucent_alpha);
		insert_color_vars!(self, "bg", theme.bg_color, theme.translucent_alpha);
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

// Parses a color from a HTML hex string
pub fn parse_color_hex(html_hex: &str) -> Option<drawing::Color> {
	if html_hex.len() == 7 {
		if let (Ok(r), Ok(g), Ok(b)) = (
			u8::from_str_radix(&html_hex[1..3], 16),
			u8::from_str_radix(&html_hex[3..5], 16),
			u8::from_str_radix(&html_hex[5..7], 16),
		) {
			return Some(drawing::Color::new(
				f32::from(r) / 255.,
				f32::from(g) / 255.,
				f32::from(b) / 255.,
				1.,
			));
		}
	} else if html_hex.len() == 9
		&& let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
			u8::from_str_radix(&html_hex[1..3], 16),
			u8::from_str_radix(&html_hex[3..5], 16),
			u8::from_str_radix(&html_hex[5..7], 16),
			u8::from_str_radix(&html_hex[7..9], 16),
		) {
		return Some(drawing::Color::new(
			f32::from(r) / 255.,
			f32::from(g) / 255.,
			f32::from(b) / 255.,
			f32::from(a) / 255.,
		));
	}
	None
}

fn get_tag_by_name<'a>(node: &roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
	node.children().find(|&child| child.tag_name().name() == name)
}

fn require_tag_by_name<'a>(node: &roxmltree::Node<'a, 'a>, name: &str) -> anyhow::Result<roxmltree::Node<'a, 'a>> {
	get_tag_by_name(node, name).ok_or_else(|| anyhow::anyhow!("Tag \"{name}\" not found"))
}

fn parse_widget_other_internal(
	template: &Rc<Template>,
	template_parameters: HashMap<Rc<str>, Rc<str>>,
	file: &ParserFile,
	ctx: &mut ParserContext,
	parent_id: WidgetID,
) -> anyhow::Result<()> {
	let template_file = ParserFile {
		document: template.node_document.clone(),
		path: file.path.clone(),
		template_parameters,
	};

	let doc = template_file.document.clone();

	let template_node = doc
		.borrow_doc()
		.get_node(template.node)
		.context("template node invalid")?;

	parse_children(&template_file, ctx, template_node, parent_id)?;

	Ok(())
}

fn parse_widget_other(
	xml_tag_name: &str,
	file: &ParserFile,
	ctx: &mut ParserContext,
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

	let template_parameters: HashMap<Rc<str>, Rc<str>> =
		attribs.iter().map(|a| (a.attrib.clone(), a.value.clone())).collect();

	parse_widget_other_internal(&template, template_parameters, file, ctx, parent_id)
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
				path = Some({
					let this = &file.path.clone();
					let include: &str = &pair.value;
					let buf = this.get_path_buf();
					let mut new_path = buf.parent().unwrap_or_else(|| Path::new("/")).to_path_buf();
					new_path.push(include);
					let new_path = normalize_path(&new_path);

					match pair.attrib.as_ref() {
						"src" => match this {
							AssetPathOwned::WguiInternal(_) => AssetPathOwned::WguiInternal(new_path),
							AssetPathOwned::BuiltIn(_) => AssetPathOwned::BuiltIn(new_path),
							AssetPathOwned::FileOrBuiltIn(_) => AssetPathOwned::FileOrBuiltIn(new_path),
							AssetPathOwned::File(_) => AssetPathOwned::File(new_path),
						},
						"src_ext" => AssetPathOwned::File(new_path),
						"src_builtin" => AssetPathOwned::BuiltIn(new_path),
						"src_internal" => AssetPathOwned::WguiInternal(new_path),
						_ => unreachable!(),
					}
				});
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
	match get_doc_from_asset_path(ctx, path_ref) {
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

pub fn replace_vars(input: &str, vars: &HashMap<Rc<str>, Rc<str>>) -> Rc<str> {
	let re = regex::Regex::new(r"\$\{([^}]*)\}").unwrap();

	/*if !vars.is_empty() {
		log::error!("template parameters {:?}", vars);
	}*/

	let out = re.replace_all(input, |captures: &regex::Captures| {
		let input_var = &captures[1];

		if let Some(replacement) = vars.get(input_var) {
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
fn process_attrib(
	template_parameters: &HashMap<Rc<str>, Rc<str>>,
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
				AttribPair::new(key, "undefined")
			}
		}
	} else {
		AttribPair::new(key, replace_vars(value, template_parameters))
	}
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
					res.push(process_attrib(&file.template_parameters, ctx, macro_key, macro_value));
				}
			} else {
				log::warn!(
					"{}: requested macro named \"{value}\" not found!",
					ctx.doc_params.path.get_str()
				);
			}
		} else {
			res.push(process_attrib(&file.template_parameters, ctx, key, value));
		}
	}

	res
}

fn parse_tag_theme<'a>(ctx: &mut ParserContext, node: roxmltree::Node<'a, 'a>) {
	for child_node in node.children() {
		let child_name = child_node.tag_name().name();
		match child_name {
			"var" => {
				parse_tag_var(ctx, child_name, child_node);
			}
			"" => { /* ignore */ }
			_ => {
				log::warn!(
					"{}: <{child_name}> is not a valid child to <theme>.",
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
	parent_node: roxmltree::Node<'a, 'a>,
	child_node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
) -> anyhow::Result<()> {
	let tag_name = child_node.tag_name().name();
	match parent_node.attribute("ignore_in_mode") {
		Some("dev") => {
			if !ctx.doc_params.extra.dev_mode {
				return Ok(()); // do not parse
			}
		}
		Some("live") => {
			if ctx.doc_params.extra.dev_mode {
				return Ok(()); // do not parse
			}
		}
		Some(s) => ctx.print_invalid_attrib(tag_name, "ignore_in_mode", s),
		_ => {}
	}

	let attribs = process_attribs(file, ctx, &child_node, false);
	let mut new_widget_id: Option<WidgetID> = None;

	match tag_name {
		"include" => {
			parse_tag_include(file, ctx, parent_id, &attribs)?;
		}
		"div" => {
			new_widget_id = Some(parse_widget_div(file, ctx, child_node, parent_id, &attribs, tag_name)?);
		}
		"rectangle" => {
			new_widget_id = Some(parse_widget_rectangle(
				file, ctx, child_node, parent_id, &attribs, tag_name,
			)?);
		}
		"label" => {
			new_widget_id = Some(parse_widget_label(
				file, ctx, child_node, parent_id, &attribs, tag_name,
			)?);
		}
		"sprite" => {
			new_widget_id = Some(parse_widget_sprite(
				file, ctx, child_node, parent_id, &attribs, tag_name,
			)?);
		}
		"image" => {
			new_widget_id = Some(parse_widget_image(
				file, ctx, child_node, parent_id, &attribs, tag_name,
			)?);
		}
		"Button" => {
			new_widget_id = Some(parse_component_button(
				file, ctx, child_node, parent_id, &attribs, tag_name,
			)?);
		}
		"Slider" => {
			new_widget_id = Some(parse_component_slider(ctx, parent_id, &attribs, tag_name)?);
		}
		"CheckBox" => {
			new_widget_id = Some(parse_component_checkbox(
				ctx,
				parent_id,
				&attribs,
				tag_name,
				CheckboxKind::CheckBox,
			)?);
		}
		"RadioBox" => {
			new_widget_id = Some(parse_component_checkbox(
				ctx,
				parent_id,
				&attribs,
				tag_name,
				CheckboxKind::RadioBox,
			)?);
		}
		"RadioGroup" => {
			new_widget_id = Some(parse_component_radio_group(
				file, ctx, child_node, parent_id, &attribs, tag_name,
			)?);
		}
		"EditBox" => new_widget_id = Some(parse_component_editbox(ctx, parent_id, &attribs, tag_name)?),
		"BarGraph" => new_widget_id = Some(parse_component_bar_graph(ctx, parent_id, &attribs, tag_name)?),
		"Tabs" => {
			new_widget_id = Some(parse_component_tabs(ctx, child_node, parent_id, &attribs, tag_name)?);
		}
		"" => { /* ignore */ }
		other_tag_name => {
			parse_widget_other(other_tag_name, file, ctx, parent_id, &attribs)?;
		}
	}

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

	Ok(())
}

fn parse_children<'a>(
	file: &ParserFile,
	ctx: &mut ParserContext,
	parent_node: roxmltree::Node<'a, 'a>,
	parent_id: WidgetID,
) -> anyhow::Result<()> {
	for child_node in parent_node.children() {
		parse_child(file, ctx, parent_node, child_node, parent_id)?;
	}

	Ok(())
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
}

// filled-in by you in `new_layout_from_assets` function
pub struct ParseDocumentParams<'a> {
	pub globals: WguiGlobals,      // mandatory field
	pub path: AssetPath<'a>,       // mandatory field
	pub extra: ParseDocumentExtra, // optional field, can be Default-ed
}

pub fn parse_from_assets(
	doc_params: &ParseDocumentParams,
	layout: &mut Layout,
	parent_id: WidgetID,
) -> anyhow::Result<ParserState> {
	let parser_data = ParserData::default();
	let mut ctx = create_default_context(doc_params, layout, &parser_data);
	ctx.populate_theme_variables();

	let (file, node_layout) = get_doc_from_asset_path(&ctx, doc_params.path)?;
	parse_document_root(&file, &mut ctx, parent_id, node_layout)?;

	// move everything essential to the result
	let result = ParserState {
		data: std::mem::take(&mut ctx.data_local),
		path: doc_params.path.to_owned(),
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

fn get_doc_from_asset_path(
	ctx: &ParserContext,
	asset_path: AssetPath,
) -> anyhow::Result<(ParserFile, roxmltree::NodeId)> {
	let data = ctx.layout.state.globals.get_asset(asset_path)?;
	let xml = String::from_utf8(data)?;

	let document = Rc::new(XmlDocument::new(xml, |xml| {
		let opt = roxmltree::ParsingOptions {
			allow_dtd: true,
			..Default::default()
		};
		roxmltree::Document::parse_with_options(xml, opt)
			.context("Unable to parse XML")
			.log_err_with(&asset_path)
			.unwrap()
	}));

	let root = document.borrow_doc().root();
	let tag_layout = require_tag_by_name(&root, "layout")?;

	let file = ParserFile {
		path: asset_path.to_owned(),
		document: document.clone(),
		template_parameters: Default::default(),
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
			"theme" => parse_tag_theme(ctx, child_node),
			"template" => parse_tag_template(file, ctx, child_node),
			"blueprint" => parse_tag_template(file, ctx, child_node),
			"macro" => parse_tag_macro(file, ctx, child_node),
			_ => {}
		}
	}

	if let Some(tag_elements) = get_tag_by_name(&node_layout, "elements") {
		parse_children(file, ctx, tag_elements, parent_id)?;
	}

	Ok(())
}

fn get_asset_path_from_kv<'a>(prefix: &'static str, key: &'a str, value: &'a str) -> AssetPath<'a> {
	let key_split = match key.find(prefix) {
		Some(pos) => {
			assert!(pos == 0, "invalid split");
			key.get(prefix.len()..).unwrap()
		}
		None => key,
	};
	match key_split {
		"src" => AssetPath::FileOrBuiltIn(value),
		"src_ext" => AssetPath::File(value),
		"src_builtin" => AssetPath::BuiltIn(value),
		"src_internal" => AssetPath::WguiInternal(value),
		other => {
			panic!("unexpected attrib {other}");
		}
	}
}
