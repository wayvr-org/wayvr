use std::{
	env, fs,
	path::{Path, PathBuf},
};

fn main() {
	let shader_dir = PathBuf::from("src/renderer_vk/shaders");
	println!("cargo:rerun-if-changed={}", shader_dir.display());

	let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
	let compiler = shaderc::Compiler::new().expect("failed to create shader compiler");
	let mut options = shaderc::CompileOptions::new().expect("failed to create shader compiler options");
	options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_3 as u32);
	options.set_include_callback({
		let shader_dir = shader_dir.clone();
		move |name, _include_type, source_name, _depth| {
			let source_path = Path::new(source_name);
			let base = source_path.parent().unwrap_or(&shader_dir);
			let mut path = base.join(name);
			if !path.exists() {
				path = shader_dir.join(name);
			}
			let content =
				fs::read_to_string(&path).map_err(|e| format!("failed reading shader include {}: {e}", path.display()))?;
			Ok(shaderc::ResolvedInclude {
				resolved_name: path.to_string_lossy().into_owned(),
				content,
			})
		}
	});

	for (name, kind) in [
		("rect.vert", shaderc::ShaderKind::Vertex),
		("rect.frag", shaderc::ShaderKind::Fragment),
		("image.vert", shaderc::ShaderKind::Vertex),
		("image.frag", shaderc::ShaderKind::Fragment),
		("text.vert", shaderc::ShaderKind::Vertex),
		("text.frag", shaderc::ShaderKind::Fragment),
	] {
		let path = shader_dir.join(name);
		let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed reading {}: {e}", path.display()));
		let artifact = compiler
			.compile_into_spirv(
				&source,
				kind,
				path.to_str().expect("shader path is UTF-8"),
				"main",
				Some(&options),
			)
			.unwrap_or_else(|e| panic!("failed compiling {}: {e}", path.display()));
		fs::write(out_dir.join(format!("{name}.spv")), artifact.as_binary_u8())
			.unwrap_or_else(|e| panic!("failed writing compiled {name}: {e}"));
	}
}
