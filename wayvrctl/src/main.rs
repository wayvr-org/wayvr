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
    packet_server::WvrProcessHandle,
};

use crate::helper::{
    WayVRClientState, wlr_input_capture, wlx_device_haptics, wlx_handsfree, wlx_input_state,
    wlx_overlay_list, wlx_overlay_set_visible, wlx_panel_modify, wlx_show_hide, wlx_switch_set,
    wlx_window_state_get, wlx_window_state_set, wvr_process_get, wvr_process_launch,
    wvr_process_list, wvr_process_terminate,
};

mod helper;
mod types;

fn main() -> ExitCode {
    env_logger::init_from_env(Env::default().default_filter_or("info"));
    let args = Args::parse();

    smol::block_on(async move {
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
    })
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
        Subcommands::WindowList { visible, hidden } => {
            // no filter: list both visible and hidden overlays
            let (visible, hidden) = match (visible, hidden) {
                (false, false) => (true, true),
                _ => (visible, hidden),
            };
            wlx_overlay_list(state, visible, hidden).await;
        }
        Subcommands::WindowSetVisible {
            overlay,
            visible_0_or_1,
        } => {
            wlx_overlay_set_visible(state, overlay, visible_0_or_1 != 0).await;
        }
        Subcommands::WindowState { overlay, command } => match command {
            WindowStateCommand::Get { what } => {
                wlx_window_state_get(state, overlay, what.into()).await;
            }
            WindowStateCommand::Set { what, value, lerp } => {
                let value = parse_window_state_value(what, &value, lerp).with_context(|| {
                    format!(
                        "Invalid value '{value}' for '{}'",
                        window_state_field_name(what)
                    )
                })?;
                wlx_window_state_set(state, overlay, what.into(), value).await;
            }
        },
        Subcommands::ProcessGet { handle } => {
            let handle =
                serde_json::from_str::<WvrProcessHandle>(&handle).context("Invalid handle")?;
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
        Subcommands::Handsfree { command } => {
            wlx_handsfree(state, command.into()).await;
        }
        Subcommands::InputCapture { command } => {
            wlr_input_capture(state, matches!(command, GrabRelease::Grab)).await;
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
    /// List WayVR overlays
    WindowList {
        /// Only list visible overlays
        #[arg(long)]
        visible: bool,
        /// Only list hidden overlays
        #[arg(long)]
        hidden: bool,
    },
    /// Set the visibility of a WayVR overlay
    WindowSetVisible {
        /// The name of the overlay
        overlay: String,
        visible_0_or_1: u8,
    },
    /// Get or set a window state property of an overlay
    WindowState {
        /// The name of the overlay
        overlay: String,
        /// Command to execute
        #[command(subcommand)]
        command: WindowStateCommand,
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
    Handsfree {
        /// Command to execute
        #[command(subcommand)]
        command: SubcommandHandsfree,
    },
    InputCapture {
        command: GrabRelease,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum GrabRelease {
    Grab,
    Release,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum PosModeEnum {
    Floating,
    Anchored,
    Static,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum HandsfreeMode {
    /// No handsfree pointer control
    None,
    /// Pointer controlled by HMD
    Hmd,
    /// Pointer controlled by HMD. Left pinch click, right pinch grab.
    HmdPinch,
    /// Pointer controlled by eye gaze
    EyeTracking,
    /// Pointer controlled eye gaze. Left pinch click, right pinch grab.
    EyeTrackingPinch,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum HandsfreeAction {
    /// The click action
    Click,
    /// The grab action
    Grab,
    /// Right-click modifier (use with click)
    RightModifier,
    /// Middle-click modifier (use with click)
    MiddleModifier,
}

#[derive(clap::Parser, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum SubcommandHandsfree {
    /// Set the handsfree mode
    SetMode { mode: HandsfreeMode },
    /// Press and hold an action
    Press { action: HandsfreeAction },
    /// Release a held action
    Release { action: HandsfreeAction },
    /// Toggle the state of an action
    Toggle { action: HandsfreeAction },
    /// Emulate a joystick scroll
    Scroll { amount: f32 },
}

#[derive(clap::Parser, Debug)]
#[allow(clippy::enum_variant_names)]
enum WindowStateCommand {
    /// Get a window state property of an overlay
    Get {
        /// The property to read
        what: WindowStateField,
    },
    /// Set a window state property of an overlay
    Set {
        /// The property to change
        what: WindowStateField,
        /// The value to set
        value: String,
        /// Lerp factor (0.0 to 1.0), used with follow_head / follow_hand positioning
        #[arg(long)]
        lerp: Option<f32>,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
enum WindowStateField {
    /// Overlay opacity (0.1 to 1.0)
    Alpha,
    /// Use additive blending when alpha < 1.0
    Additive,
    /// Whether this overlay reacts to grab action
    Grabbable,
    /// Whether laser pointers hit or pass through this overlay
    Interactable,
    /// Overlay positioning: floating, anchored, static, follow_head, follow_hand_left, follow_hand_right
    Positioning,
    /// Screen curvature, 0 is disabled
    Curvature,
    /// Whether hovering this overlay will block inputs to other VR apps
    #[value(alias = "block-input")]
    BlockInput,
    /// Whether the overlay billboards towards the HMD
    #[value(alias = "align-to-hmd")]
    AlignToHmd,
    /// Whether the overlay is shown on all sets (unaffected by set switching)
    Global,
}

fn window_state_field_name(field: WindowStateField) -> &'static str {
    match field {
        WindowStateField::Alpha => "alpha",
        WindowStateField::Grabbable => "grabbable",
        WindowStateField::Interactable => "interactable",
        WindowStateField::Positioning => "positioning",
        WindowStateField::Curvature => "curvature",
        WindowStateField::Additive => "additive",
        WindowStateField::BlockInput => "block_input",
        WindowStateField::AlignToHmd => "align_to_hmd",
        WindowStateField::Global => "global",
    }
}

fn parse_window_state_value(
    field: WindowStateField,
    raw: &str,
    lerp: Option<f32>,
) -> anyhow::Result<packet_client::WlxWindowStateValue> {
    use packet_client::WlxWindowStateValue as Value;

    let parse_bool = |raw: &str| -> anyhow::Result<bool> {
        match raw {
            "0" | "false" | "off" => Ok(false),
            "1" | "true" | "on" => Ok(true),
            _ => anyhow::bail!("expected 0 or 1"),
        }
    };

    Ok(match field {
        WindowStateField::Alpha => {
            let alpha = raw.parse::<f32>()?;
            if !(0.1..=1.0).contains(&alpha) {
                anyhow::bail!("expected a value between 0.1 and 1.0");
            }
            Value::Float(alpha)
        }
        WindowStateField::Grabbable => Value::Bool(parse_bool(raw)?),
        WindowStateField::Interactable => Value::Bool(parse_bool(raw)?),
        WindowStateField::Positioning => Value::Positioning(parse_positioning(raw, lerp)?),
        WindowStateField::Curvature => Value::Float(raw.parse::<f32>()?),
        WindowStateField::Additive => Value::Bool(parse_bool(raw)?),
        WindowStateField::BlockInput => Value::Bool(parse_bool(raw)?),
        WindowStateField::AlignToHmd => Value::Bool(parse_bool(raw)?),
        WindowStateField::Global => Value::Bool(parse_bool(raw)?),
    })
}

fn parse_positioning(
    raw: &str,
    lerp: Option<f32>,
) -> anyhow::Result<packet_client::WlxPositioning> {
    let lerp = lerp.unwrap_or(0.0);

    Ok(match raw {
        "floating" => packet_client::WlxPositioning::Floating,
        "anchored" => packet_client::WlxPositioning::Anchored,
        "static" => packet_client::WlxPositioning::Static,
        "follow_head" | "follow-head" => packet_client::WlxPositioning::FollowHead { lerp },
        "follow_hand_left" | "follow-hand-left" => packet_client::WlxPositioning::FollowHand {
            hand: packet_client::WlxHand::Left,
            lerp,
        },
        "follow_hand_right" | "follow-hand-right" => packet_client::WlxPositioning::FollowHand {
            hand: packet_client::WlxHand::Right,
            lerp,
        },
        _ => anyhow::bail!(
            "expected floating, anchored, static, follow_head or follow_hand_<left|right>"
        ),
    })
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
