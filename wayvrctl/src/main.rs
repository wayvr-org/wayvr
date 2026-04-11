use std::{
    collections::HashMap,
    process::{self, ExitCode},
    time::Duration,
};

use anyhow::Context;
use clap::Parser;
use env_logger::Env;
use wayvr_ipc::{
    client::WayVRClient,
    ipc,
    packet_client::{self, PositionMode},
};

use crate::helper::{
    WayVRClientState, wlx_device_haptics, wlx_input_state, wlx_panel_modify,
    wlx_screen_focus_toggle, wlx_show_hide, wlx_switch_set, wvr_process_get, wvr_process_launch,
    wvr_process_list, wvr_process_terminate, wvr_window_list, wvr_window_set_visible,
};

mod helper;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    env_logger::init_from_env(Env::default().default_filter_or("info"));
    let args = Args::parse();

    let mut state = WayVRClientState {
        wayvr_client: WayVRClient::new(&format!("wayvrctl-{}", process::id()))
            .await
            .inspect_err(|e| {
                log::error!("Failed to initialize WayVR connection: {e:?}");
                process::exit(1);
            })
            .unwrap(),
        serial_generator: ipc::SerialGenerator::new(),
        pretty_print: args.pretty,
    };

    let maybe_err = if let Subcommands::Batch { fail_fast } = args.command {
        run_batch(&mut state, fail_fast).await
    } else {
        run_once(&mut state, args).await
    };

    if let Err(e) = maybe_err {
        log::error!("{e:?}");
        return ExitCode::FAILURE;
    } else {
        std::thread::sleep(Duration::from_millis(20));
    }

    ExitCode::SUCCESS
}

async fn run_batch(state: &mut WayVRClientState, fail_fast: bool) -> anyhow::Result<()> {
    let stdin = std::io::stdin();

    for (line_no, line) in stdin.lines().enumerate() {
        let line = line.context("error reading stdin")?;

        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }

        if let Err(e) = parse_run_line(state, &line)
            .await
            .with_context(|| format!("error on line {}", line_no + 1))
        {
            if fail_fast {
                return Err(e);
            } else {
                log::error!("{e:?}");
            }
        }
    }
    Ok(())
}

async fn parse_run_line(state: &mut WayVRClientState, line: &str) -> anyhow::Result<()> {
    let mut argv = shell_words::split(line).context("parse error")?;

    // clap expects argv[0] to be the binary name
    argv.insert(0, env!("CARGO_PKG_NAME").to_string());

    let args = Args::try_parse_from(argv).context("invalid arguments")?;
    run_once(state, args).await?;

    Ok(())
}

async fn run_once(state: &mut WayVRClientState, args: Args) -> anyhow::Result<()> {
    match args.command {
        Subcommands::Batch { .. } => {
            log::warn!("Ignoring recursive batch command");
        }
        Subcommands::InputState => {
            wlx_input_state(state).await;
        }
        Subcommands::WindowList => {
            wvr_window_list(state).await;
        }
        Subcommands::WindowSetVisible {
            handle,
            visible_0_or_1,
        } => {
            let handle = serde_json::from_str(&handle).context("Invalid handle")?;
            wvr_window_set_visible(state, handle, visible_0_or_1 != 0).await;
        }
        Subcommands::ProcessGet { handle } => {
            let handle = serde_json::from_str(&handle).context("Invalid handle")?;
            wvr_process_get(state, handle).await;
        }
        Subcommands::ProcessList => {
            wvr_process_list(state).await;
        }
        Subcommands::ProcessTerminate { handle } => {
            let handle = serde_json::from_str(&handle).context("Invalid handle")?;
            wvr_process_terminate(state, handle).await;
        }
        Subcommands::ProcessLaunch {
            exec,
            name,
            env,
            resolution,
            pos,
            icon,
            args,
        } => {
            let env = env.split(",").map(|s| s.to_string()).collect::<Vec<_>>();
            let resolution = resolution
                .split_once('x')
                .and_then(|(x, y)| Some([x.parse::<u32>().ok()?, y.parse::<u32>().ok()?]))
                .context(
                    "Invalid resolution format. Expecting <width>x<height>, for example: 1920x1080, 1280x720",
                )?;

            let pos_mode = match pos {
                PosModeEnum::Floating => PositionMode::Float,
                PosModeEnum::Static => PositionMode::Static,
                PosModeEnum::Anchored => PositionMode::Anchor,
            };

            wvr_process_launch(
                state,
                exec,
                name,
                env,
                resolution,
                pos_mode,
                icon,
                args,
                HashMap::new(),
            )
            .await;
        }
        Subcommands::Haptics {
            device,
            intensity,
            duration,
            frequency,
        } => {
            wlx_device_haptics(state, device, intensity, duration, frequency).await;
        }
        Subcommands::ShowHide => {
            wlx_show_hide(state).await;
        }
        Subcommands::PanelModify {
            overlay,
            element,
            command,
        } => {
            let command = match command {
                SubcommandPanelModify::SetText { text } => {
                    packet_client::WlxModifyPanelCommand::SetText(text.join(" "))
                }
                SubcommandPanelModify::SetColor { hex_color } => {
                    packet_client::WlxModifyPanelCommand::SetColor(hex_color)
                }
                SubcommandPanelModify::SetImage { absolute_path } => {
                    packet_client::WlxModifyPanelCommand::SetImage(absolute_path)
                }
                SubcommandPanelModify::SetVisible { visible_0_or_1 } => {
                    packet_client::WlxModifyPanelCommand::SetVisible(visible_0_or_1 != 0)
                }
                SubcommandPanelModify::SetStickyState {
                    sticky_state_0_or_1,
                } => packet_client::WlxModifyPanelCommand::SetStickyState(sticky_state_0_or_1 != 0),
            };

            wlx_panel_modify(state, overlay, element, command).await;
        }
        Subcommands::SwitchSet { set_or_0: set } => {
            let set = if set == 0 { None } else { Some((set - 1) as _) };
            wlx_switch_set(state, set).await;
        }
        Subcommands::ScreenFocusToggle { screen_name } => {
            wlx_screen_focus_toggle(state, screen_name, 0.5, 0.5, None, false).await;
        }
        Subcommands::ScreenFocusAt {
            screen_name,
            target_x,
            target_y,
            crop_x,
            crop_y,
            crop_w,
            crop_h,
            refresh_only,
        } => {
            let crop_rect = crop_x
                .zip(crop_y)
                .zip(crop_w)
                .zip(crop_h)
                .map(|(((x, y), w), h)| [x, y, w, h]);
            wlx_screen_focus_toggle(
                state,
                screen_name,
                target_x,
                target_y,
                crop_rect,
                refresh_only,
            )
            .await;
        }
    }
    Ok(())
}

/// A command-line interface for WayVR IPC
#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The command to run
    #[command(subcommand)]
    command: Subcommands,

    /// Pretty-print JSON output
    #[arg(short, long)]
    pretty: bool,
}

#[derive(clap::Parser, Debug)]
enum Subcommands {
    /// Read commands from stdout, one per line.
    Batch {
        /// Stop on the first error
        #[arg(short, long)]
        fail_fast: bool,
    },
    /// Get the positions of HMD & controllers
    InputState,
    /// List WayVR windows
    WindowList,
    /// Delete a WayVR display
    // DisplaySetLayout skipped
    /// Change the visibility of a window on a WayVR display
    WindowSetVisible {
        /// A JSON window handle returned by DisplayWindowList
        handle: String,
        visible_0_or_1: u8,
    },
    /// Retrieve information about a WayVR-managed process
    ProcessGet {
        /// A JSON process handle returned by ProcessList or ProcessLaunch
        handle: String,
    },
    /// List all processes managed by WayVR
    ProcessList,
    /// Terminate a WayVR-managed process
    ProcessTerminate {
        /// A JSON process handle returned by ProcessList or ProcessLaunch
        handle: String,
    },
    /// Launch a new process inside WayVR
    ProcessLaunch {
        /// Name for the overlay
        #[arg(short, long, default_value = "")]
        name: String,
        /// Environment variables, separated by comma
        #[arg(short, long, default_value = "")]
        env: String,
        /// Executable to run
        exec: String,
        #[arg(default_value = "1920x1080")]
        resolution: String,
        /// Default positioning
        pos: PosModeEnum,
        /// Absolute path to the app icon
        icon: Option<String>,
        /// Arguments to pass to executable
        #[arg(default_value = "")]
        args: String,
    },
    /// Trigger haptics on the user's controller
    Haptics {
        /// 0 for left, 1 for right controller
        device: usize,
        #[arg(short, long, default_value = "0.25")]
        intensity: f32,
        #[arg(short, long, default_value = "0.1")]
        duration: f32,
        #[arg(short, long, default_value = "0.1")]
        frequency: f32,
    },
    /// Toggle overlay show or hide
    ShowHide,
    /// Apply a modification to a panel element
    PanelModify {
        /// The name of the overlay (XML file name without extension)
        overlay: String,
        /// The id of the element to modify, as set in the XML
        element: String,
        /// Command to execute
        #[command(subcommand)]
        command: SubcommandPanelModify,
    },
    SwitchSet {
        /// Set number to switch to, 0 to hide all sets
        set_or_0: usize,
    },
    ScreenFocusToggle {
        screen_name: String,
    },
    ScreenFocusAt {
        screen_name: String,
        target_x: f32,
        target_y: f32,
        #[arg(long)]
        crop_x: Option<f32>,
        #[arg(long)]
        crop_y: Option<f32>,
        #[arg(long)]
        crop_w: Option<f32>,
        #[arg(long)]
        crop_h: Option<f32>,
        #[arg(long)]
        refresh_only: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum PosModeEnum {
    Floating,
    Anchored,
    Static,
}

#[derive(clap::Parser, Debug)]
#[allow(clippy::enum_variant_names)]
enum SubcommandPanelModify {
    /// Set the text of a <label> or <Button>
    SetText {
        /// Text that needs to be set
        #[arg(num_args = 1.., action = clap::ArgAction::Append)]
        text: Vec<String>,
    },
    /// Set the color of a <rectangle> or <label> or monochrome <sprite>
    SetColor {
        /// Color in HTML hex format (#rrggbb or #rrggbbaa)
        hex_color: String,
    },
    /// Set the content of a <sprite> or <image>. Max size for <sprite> is 256x256.
    SetImage {
        /// Absolute path to a svg, gif, png, jpeg or webp image.
        absolute_path: String,
    },
    /// Set the visibility of a <div>, <rectangle>, <label>, <sprite> or <image>
    SetVisible { visible_0_or_1: u8 },
    /// Set the sticky state of a <Button>. Intended for buttons without `sticky="1"`.
    SetStickyState { sticky_state_0_or_1: u8 },
}
