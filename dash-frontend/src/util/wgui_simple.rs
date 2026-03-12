use wgui::{
	i18n::Translation,
	layout::{Layout, WidgetID},
	renderer_vk::text::TextStyle,
	widget::label::{WidgetLabel, WidgetLabelParams},
};

#[allow(dead_code)]
pub fn create_label(layout: &mut Layout, parent: WidgetID, content: Translation) -> anyhow::Result<()> {
	let label = WidgetLabel::create(
		&mut layout.state,
		WidgetLabelParams {
			content,
			style: TextStyle {
				wrap: true,
				..Default::default()
			},
		},
	);

	layout.add_child(parent, label, Default::default())?;

	Ok(())
}
