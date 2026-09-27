use regex::Regex;
use shaderc::{Compiler, EnvVersion, ShaderKind, TargetEnv};
use std::{path::PathBuf, process::Command};

const SHADERS: &[(&str, ShaderKind)] = &[
    ("quad.vert", ShaderKind::Vertex),
    ("color.frag", ShaderKind::Fragment),
    ("grid.frag", ShaderKind::Fragment),
    ("screen.frag", ShaderKind::Fragment),
    ("simple.frag", ShaderKind::Fragment),
    ("srgb.frag", ShaderKind::Fragment),
    ("sky.frag", ShaderKind::Fragment),
];

fn main() {
    compile_shaders();

    let mut wlx_build = get_version().unwrap_or(format!("{}-unknown", env!("CARGO_PKG_VERSION")));

    match std::env::var("GITHUB_JOB").as_deref() {
        Ok("make_release") => {
            wlx_build = format!("{} (Release)", wlx_build);
        }
        Ok("build_appimage") => {
            wlx_build = format!("{} (AppImage)", wlx_build);
        }
        _ => {}
    }
    println!("cargo:rustc-env=WLX_BUILD={wlx_build}");
}

fn compile_shaders() {
    let shader_dir = PathBuf::from("src/shaders");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let compiler = Compiler::new().expect("shaderc compiler is available");

    let mut options = shaderc::CompileOptions::new().expect("shaderc options are available");
    options.set_target_env(TargetEnv::Vulkan, EnvVersion::Vulkan1_3 as u32);

    for (name, kind) in SHADERS {
        let path = shader_dir.join(name);
        println!("cargo:rerun-if-changed={}", path.display());

        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let artifact = compiler
            .compile_into_spirv(&source, *kind, name, "main", Some(&options))
            .unwrap_or_else(|e| panic!("failed to compile {}: {e}", path.display()));
        std::fs::write(out_dir.join(format!("{name}.spv")), artifact.as_binary_u8())
            .unwrap_or_else(|e| panic!("failed to write compiled shader {name}: {e}"));
    }
}

fn get_version() -> Result<String, Box<dyn std::error::Error>> {
    let re = Regex::new(r"v([0-9.]+)-([0-9]+)-g([a-f0-9]+)").unwrap(); // safe
    let output = Command::new("git")
        .args(["describe", "--tags", "--abbrev=7", "--dirty"])
        .output()?;

    let mut output_str = String::from_utf8(output.stdout)?;

    if output_str.is_empty() {
        let output = Command::new("git")
            .args(["describe", "--tags", "--abbrev=7", "--dirty", "--always"])
            .output()?;

        output_str = format!(
            "{}-{}",
            env!("CARGO_PKG_VERSION"),
            String::from_utf8(output.stdout)?
        );
    }

    Ok(re.replace_all(&output_str, "${1}.r${2}.${3}").into_owned())
}
