use std::sync::Arc;

use crate::backend::wayvr::{self, WayVRTask, WvrServerState};
use wayvr_ipc::packet_client::{HandsfreeMode, HandsfreeParams};
use wayvr_ipc::packet_server::PacketServer;
use wlx_common::config::HandsfreePointer;

use crate::{
    backend::{
        self,
        task::{InputTask, OverlayTask, TaskType, ToggleMode},
    },
    ipc::{signal::WayVRSignal, window_state},
    state::AppState,
    windowing::{OverlaySelector, manager::OverlayWindowManager},
};

fn process_tick_tasks(
    tick_tasks: Vec<backend::wayvr::TickTask>,
    server_state: &mut WvrServerState,
) {
    for tick_task in tick_tasks {
        match tick_task {
            backend::wayvr::TickTask::NewExternalProcess(request) => {
                log::info!("Registering external process with PID {}", request.pid);
                server_state.add_external_process(request.pid);
                server_state.manager.add_client(wayvr::client::WayVRClient {
                    client: request.client,
                    pid: request.pid,
                });
            }
        }
    }
}

pub fn tick_events<O>(
    app: &mut AppState,
    overlays: &mut OverlayWindowManager<O>,
) -> anyhow::Result<()>
where
    O: Default,
{
    while let Some(signal) = app.wayvr_signals.read() {
        match signal {
            WayVRSignal::WindowVisibilityChanged(window_handle, visible) => {
                if let Some(server) = &mut app.wvr_server {
                    server
                        .tasks
                        .send(WayVRTask::VisibilityChange(window_handle, visible));
                }
            }
            WayVRSignal::DeviceHaptics(device, haptics) => {
                app.tasks
                    .enqueue(TaskType::Input(InputTask::Haptics { device, haptics }));
            }
            WayVRSignal::ShowHide => {
                app.tasks.enqueue(TaskType::Overlay(OverlayTask::ShowHide));
            }
            WayVRSignal::SwitchSet(set) => {
                app.tasks
                    .enqueue(TaskType::Overlay(OverlayTask::SwitchSet(set)));
            }
            WayVRSignal::CustomTask(custom_task) => {
                app.tasks
                    .enqueue(TaskType::Overlay(OverlayTask::ModifyPanel(custom_task)));
            }
            WayVRSignal::GetWindowState(connection_id, serial, get_params) => {
                let result = window_state::get_state(overlays, &get_params);
                app.ipc_server.send_response(
                    connection_id,
                    &PacketServer::WlxWindowStateGetResponse(serial, result),
                );
            }
            WayVRSignal::ListOverlays(connection_id, serial, list_params) => {
                let list = overlays.list_overlays(list_params.visible, list_params.hidden);
                app.ipc_server.send_response(
                    connection_id,
                    &PacketServer::WlxOverlayListResponse(
                        serial,
                        wayvr_ipc::packet_server::WlxOverlayList { list },
                    ),
                );
            }
            WayVRSignal::SetWindowState(set_params) => {
                let name: Arc<str> = set_params.overlay.clone().into();
                let field = set_params.field;
                let value = set_params.value;

                app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                    OverlaySelector::Name(name),
                    Box::new(move |_app, config| window_state::set_field(config, field, value)),
                )));
            }
            WayVRSignal::SetOverlayVisible(overlay, visible) => {
                let name: Arc<str> = overlay.into();
                let mode = if visible {
                    ToggleMode::EnsureOn
                } else {
                    ToggleMode::EnsureOff
                };

                app.tasks
                    .enqueue(TaskType::Overlay(OverlayTask::ToggleOverlay(
                        OverlaySelector::Name(name),
                        mode,
                    )));
            }
            WayVRSignal::Handsfree(params) => match params {
                HandsfreeParams::SetMode(mode) => {
                    let mode = match mode {
                        HandsfreeMode::None => HandsfreePointer::None,
                        HandsfreeMode::Hmd => HandsfreePointer::HmdOnly,
                        HandsfreeMode::HmdPinch => HandsfreePointer::Hmd,
                        HandsfreeMode::EyeTracking => HandsfreePointer::EyeTrackingOnly,
                        HandsfreeMode::EyeTrackingPinch => HandsfreePointer::EyeTracking,
                    };
                    app.session.config.handsfree_pointer = mode;
                }
                _ => app.input_state.apply_handsfree_action(params),
            },
        }
    }

    let tick_tasks = WvrServerState::tick_events(app)?;
    if let Some(wayvr_server) = app.wvr_server.as_mut() {
        process_tick_tasks(tick_tasks, wayvr_server);
    }

    Ok(())
}
