#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(
	clippy::suboptimal_flops,
	clippy::cast_precision_loss,
	clippy::missing_errors_doc,
	clippy::default_trait_access,
	clippy::missing_panics_doc,
	clippy::cast_possible_wrap,
	clippy::cast_possible_truncation,
	clippy::cast_sign_loss,
	clippy::items_after_statements,
	clippy::future_not_send,
	clippy::must_use_candidate,
	clippy::implicit_hasher,
	clippy::option_if_let_else,
	clippy::significant_drop_tightening,
	clippy::float_cmp,
	clippy::needless_pass_by_ref_mut,
	clippy::use_self,
	clippy::match_same_arms,
	clippy::too_many_lines,
	clippy::struct_excessive_bools
)]

pub mod animation;
pub mod any;
pub mod assets;
mod assets_internal;
pub mod components;
pub mod drawing;
pub mod event;
pub mod font_config;
pub mod gfx;
pub mod globals;
pub mod i18n;
pub mod layout;
pub mod log;
pub mod parser;
pub mod renderer_vk;
pub mod sound;
pub mod stack;
pub mod task;
pub mod theme;
pub mod widget;
pub mod windowing;

// re-exported libs
pub use cosmic_text;
pub use taffy;
