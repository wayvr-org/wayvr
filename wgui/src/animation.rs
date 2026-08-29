use glam::{FloatExt, Mat4, Vec2, Vec3};

use crate::{
	drawing::Boundary,
	event::{CallbackDataCommon, EventAlterables},
	layout::{LayoutState, WidgetID},
	widget::{WidgetData, WidgetObj, label::WidgetLabel},
};

pub enum AnimationEasing {
	Linear,
	InQuad,   // ^2
	InCubic,  // ^3
	InQuint,  // ^5
	OutQuad,  // ^2
	OutCubic, // ^3
	OutQuint, // ^5
	OutBack,
	InBack,
}

impl AnimationEasing {
	pub fn interpolate(&self, x: f32) -> f32 {
		match self {
			Self::Linear => x,
			Self::InQuad => x.powi(2),
			Self::InCubic => x.powi(3),
			Self::InQuint => x.powi(5),
			Self::OutQuad => 1.0 - (1.0 - x).powi(2),
			Self::OutCubic => 1.0 - (1.0 - x).powi(3),
			Self::OutQuint => 1.0 - (1.0 - x).powi(5),
			Self::OutBack => {
				let a = 1.7;
				let b = a + 1.0;
				1.0 + b * (x - 1.0).powi(3) + a * (x - 1.0).powi(2)
			}
			Self::InBack => {
				let a = 1.7;
				let b = a + 1.0;
				b * x.powi(3) - a * x.powi(2)
			}
		}
	}
}

pub struct CallbackData<'a> {
	pub obj: &'a mut dyn WidgetObj,
	pub data: &'a mut WidgetData,
	pub widget_id: WidgetID,
	pub widget_boundary: Boundary,
	pub pos: f32, // 0.0 (start of animation) - 1.0 (end of animation)
	pub stop_me: &'a mut bool,
}

pub type AnimationCallback = Box<dyn Fn(&mut CallbackDataCommon, &mut CallbackData)>;

#[derive(Clone, Copy)]
pub enum AnimationDuration {
	Seconds(f32), // multiplied by animation_mult
	SecondsFixed(f32),
	Infinity,
}

impl AnimationDuration {
	pub fn to_ticks(&self, ticks_per_seconds: u32, animation_mult: f32) -> u32 {
		match self {
			AnimationDuration::Seconds(secs) => (secs * ticks_per_seconds as f32 * animation_mult) as u32,
			AnimationDuration::SecondsFixed(secs) => (secs * ticks_per_seconds as f32) as u32,
			AnimationDuration::Infinity => u32::MAX,
		}
	}
}

struct Ticks {
	remaining: u32,
	duration: u32,
}

pub struct Animation {
	target_widget: WidgetID,

	id: u32,
	duration: AnimationDuration,

	// filled-in at first iteration
	ticks: Option<Ticks>,

	easing: AnimationEasing,

	pos: f32,
	pos_prev: f32,
	last_tick: bool,

	callback: AnimationCallback,
}

impl Animation {
	pub fn new(
		target_widget: WidgetID,
		duration: AnimationDuration,
		easing: AnimationEasing,
		callback: AnimationCallback,
	) -> Self {
		Self::new_ex(target_widget, 0, duration, easing, callback)
	}

	pub fn effect_slide(
		target_widget: WidgetID,
		duration: AnimationDuration,
		easing: AnimationEasing,
		dir: Vec2,
	) -> Self {
		Animation::new(
			target_widget,
			duration,
			easing,
			Box::new(move |common, data| {
				data.data.transform =
					Mat4::from_translation(Vec3::new((1.0 - data.pos) * dir.x, (1.0 - data.pos) * dir.y, 0.0));
				common.alterables.mark_redraw();
			}),
		)
	}

	pub fn effect_label_fade_in(target_widget: WidgetID, duration: AnimationDuration, easing: AnimationEasing) -> Self {
		Animation::new(
			target_widget,
			duration,
			easing,
			Box::new(move |common, data| {
				let Ok(label) = data.obj.cast_mut::<WidgetLabel>() else {
					debug_assert!(false); // cast failed
					return;
				};
				label.set_color(common, label.get_color().with_alpha(data.pos), true);
			}),
		)
	}

	pub fn new_ex(
		target_widget: WidgetID,
		animation_id: u32,
		duration: AnimationDuration,
		easing: AnimationEasing,
		callback: AnimationCallback,
	) -> Self {
		Self {
			target_widget,
			id: animation_id,
			callback,
			easing,
			duration,
			ticks: None,
			last_tick: false,
			pos: 0.0,
			pos_prev: 0.0,
		}
	}

	/// @returns false if it wants to be stopped
	#[must_use]
	fn call(&self, state: &LayoutState, alterables: &mut EventAlterables, pos: f32) -> bool {
		let Some(widget) = state.widgets.get(self.target_widget).cloned() else {
			return false; // failed
		};

		let mut widget_state = widget.state();
		let (data, obj) = widget_state.get_data_obj_mut();
		let mut stop_me = false;

		let data = &mut CallbackData {
			widget_id: self.target_widget,
			widget_boundary: state.get_widget_boundary(self.target_widget),
			obj,
			data,
			pos,
			stop_me: &mut stop_me,
		};

		let common = &mut CallbackDataCommon { state, alterables };

		(self.callback)(common, data);

		!stop_me
	}
}

#[derive(Default)]
pub struct Animations {
	running_animations: Vec<Animation>,
}

impl Animations {
	pub fn tick(&mut self, state: &LayoutState, alterables: &mut EventAlterables) {
		for anim in &mut self.running_animations {
			let mut ticks = anim.ticks.take().unwrap_or_else(|| {
				let ticks = anim
					.duration
					.to_ticks(state.ticks_per_seconds, state.theme.animation_mult);
				Ticks {
					remaining: ticks,
					duration: ticks,
				}
			});

			let x = 1.0 - (ticks.remaining as f32 / ticks.duration as f32);
			let pos = if ticks.remaining > 0 {
				anim.easing.interpolate(x)
			} else {
				anim.last_tick = true;
				1.0
			};

			anim.pos_prev = anim.pos;
			anim.pos = pos;

			if anim.last_tick {
				let _ = anim.call(state, alterables, 1.0);
				alterables.needs_redraw = true;
			} else {
				ticks.remaining -= 1;
			}

			anim.ticks = Some(ticks);
		}

		self.running_animations.retain(|anim| !anim.last_tick);
	}

	pub fn process(&mut self, state: &LayoutState, alterables: &mut EventAlterables, alpha: f32) {
		for anim in &mut self.running_animations {
			let pos = anim.pos_prev.lerp(anim.pos, alpha);
			if !anim.call(state, alterables, pos)
				&& let Some(ticks) = &mut anim.ticks
			{
				ticks.remaining = 0;
			}
		}
	}

	pub fn add(&mut self, anim: Animation, state: &LayoutState, alterables: &mut EventAlterables) {
		// prevent running two animations at once
		self.stop_by_widget(anim.target_widget, Some(anim.id));

		// call the animation for the first time with pos 0.0
		if !anim.call(state, alterables, 0.0) {
			return;
		}

		self.running_animations.push(anim);
	}

	pub fn stop_by_widget(&mut self, widget_id: WidgetID, opt_animation_id: Option<u32>) {
		self.running_animations.retain(|anim| {
			if let Some(animation_id) = &opt_animation_id {
				if anim.target_widget == widget_id {
					anim.id != *animation_id
				} else {
					true
				}
			} else {
				anim.target_widget != widget_id
			}
		});
	}
}
