use glam::{Mat4, Vec3};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use wgui::{
	animation::{Animation, AnimationDuration, AnimationEasing},
	color::{WguiColor, WguiColorName},
	components::tooltip::{TOOLTIP_BORDER_COLOR, TOOLTIP_COLOR},
	i18n::Translation,
	layout::{Layout, LayoutTask, LayoutTasks, WidgetID},
	renderer_vk::{
		text::{FontWeight, HorizontalAlign, TextStyle},
		util::centered_matrix,
	},
	taffy::{
		self,
		prelude::{auto, length, percent},
	},
	widget::{
		div::WidgetDiv,
		label::{WidgetLabel, WidgetLabelParams},
		rectangle::{WidgetRectangle, WidgetRectangleParams},
		util::WLength,
	},
};

struct MountedToast {
	#[allow(dead_code)]
	id_root: WidgetID, // decorations of a toast
	layout_tasks: LayoutTasks,
}

struct State {
	toast: Option<MountedToast>,
	queue: VecDeque<Translation>,
	timeout: u32, // in ticks
}

pub struct ToastManager {
	state: Rc<RefCell<State>>,
	needs_tick: bool,
}

impl Drop for MountedToast {
	fn drop(&mut self) {
		self.layout_tasks.push(LayoutTask::RemoveWidget(self.id_root));
	}
}

fn def_toast_duration() -> AnimationDuration {
	AnimationDuration::Seconds(2.5)
}

impl ToastManager {
	pub fn new() -> Self {
		Self {
			state: Rc::new(RefCell::new(State {
				toast: None,
				timeout: 0,
				queue: VecDeque::new(),
			})),
			needs_tick: false,
		}
	}

	fn mount_toast(&self, layout: &mut Layout, state: &mut State, content: Translation) -> anyhow::Result<()> {
		let (root, _) = layout.add_topmost_child(
			WidgetDiv::create(),
			taffy::Style {
				position: taffy::Position::Absolute,
				size: taffy::Size {
					width: percent(1.0_f32),
					height: percent(0.8_f32),
				},
				align_items: Some(taffy::AlignItems::END),
				justify_content: Some(taffy::JustifyContent::CENTER),
				..Default::default()
			},
		)?;

		let (rect, _) = layout.add_child(
			root.id,
			WidgetRectangle::create(WidgetRectangleParams {
				color: TOOLTIP_COLOR.into(),
				border_color: TOOLTIP_BORDER_COLOR.into(),
				border: 2.0,
				round: WLength::Percent(1.0),
				..Default::default()
			}),
			taffy::Style {
				position: taffy::Position::Relative,
				gap: length(4.0_f32),
				padding: taffy::Rect {
					left: length(16.0_f32),
					right: length(16.0_f32),
					top: length(8.0_f32),
					bottom: length(8.0_f32),
				},
				max_size: taffy::Size {
					width: length(400.0_f32),
					height: auto(),
				},
				..Default::default()
			},
		)?;

		let label = WidgetLabel::create(
			&mut layout.state,
			WidgetLabelParams {
				content,
				style: TextStyle {
					weight: Some(FontWeight::Bold),
					align: Some(HorizontalAlign::Center),
					wrap: true,
					..Default::default()
				},
				..Default::default()
			},
		);
		let (label, _) = layout.add_child(rect.id, label, taffy::Style { ..Default::default() })?;

		// show-up animation
		layout.alterables.animate(Animation::new(
			rect.id,
			def_toast_duration(),
			AnimationEasing::Linear,
			Box::new(move |common, data| {
				let pos_showup = AnimationEasing::OutQuint.interpolate((data.pos * 4.0).min(1.0));
				let opacity = 1.0 - AnimationEasing::OutQuint.interpolate(((data.pos - 0.75) * 4.0).clamp(0.0, 1.0));
				let scale = AnimationEasing::OutBack.interpolate((data.pos * 4.0).min(1.0));

				{
					let mtx = Mat4::from_translation(Vec3::new(0.0, (1.0 - pos_showup) * 20.0, 0.0))
						* Mat4::from_scale(Vec3::new(scale, scale, 1.0));
					data.data.transform = centered_matrix(data.widget_boundary.size, &mtx);
				}

				let rect = data.obj.get_as_mut::<WidgetRectangle>().unwrap();
				rect.params.color = rect.params.color.with_alpha(opacity);
				rect.params.border_color = rect.params.border_color.with_alpha(opacity);

				let mut label = common.state.widgets.get_as::<WidgetLabel>(label.id).unwrap();
				label.set_color(
					common,
					WguiColor::from(WguiColorName::OnBackgroundVariant).with_alpha(opacity),
					true,
				);
				common.alterables.mark_redraw();
			}),
		));

		state.toast = Some(MountedToast {
			id_root: root.id,
			layout_tasks: layout.tasks.clone(),
		});

		Ok(())
	}

	pub fn tick(&mut self, layout: &mut Layout) -> anyhow::Result<()> {
		let mut state = self.state.borrow_mut();
		if state.timeout > 0 {
			state.timeout -= 1;
		}

		if !self.needs_tick {
			return Ok(());
		}

		if state.timeout == 0 {
			state.toast = None;
			state.timeout = def_toast_duration().to_ticks(layout.state.ticks_per_seconds, layout.state.theme.animation_mult);
			// mount next
			if let Some(content) = state.queue.pop_front() {
				self.mount_toast(layout, &mut state, content)?;
			}
		}

		if state.queue.is_empty() && state.toast.is_none() {
			self.needs_tick = false;
		}

		Ok(())
	}

	pub fn push(&mut self, content: Translation) {
		let mut state = self.state.borrow_mut();
		state.queue.push_back(content);
		self.needs_tick = true;
	}
}
