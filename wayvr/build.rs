use regex::Regex;
use std::process::Command;

fn main() {
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
    println!("cargo:rustc-env=WLX_BUILD={}", wlx_build);
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
