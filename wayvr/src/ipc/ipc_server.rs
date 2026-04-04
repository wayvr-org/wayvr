use crate::backend::wayvr::{self, WvrServerState};

use crate::{
    backend::input::InputState,
    ipc::{event_queue::SyncEventQueue, signal::WayVRSignal},
};
use bytes::BufMut;
use glam::Vec3A;
use interprocess::local_socket::{self, ToNsName, traits::Listener};
use smallvec::SmallVec;
use std::io::{Read, Write};
use wayvr_ipc::{
    ipc::{self},
    packet_client::{self, PacketClient},
    packet_server::{self, PacketServer, WlxInputStatePointer},
};

#[allow(dead_code)] // not used for now, wayvr accepts any client
pub struct AuthInfo {
    pub client_name: String,
    pub protocol_version: u32, // client protocol version
}

pub struct Connection {
    alive: bool,
    conn: local_socket::Stream,
    next_packet: Option<u32>,
    auth: Option<AuthInfo>,
}

pub fn send_packet(conn: &mut local_socket::Stream, data: &[u8]) -> anyhow::Result<()> {
    let mut bytes = bytes::BytesMut::new();

    // packet size
    bytes.put_u32(data.len() as u32);

    // packet data
    bytes.put_slice(data);

    conn.write_all(&bytes)?;

    Ok(())
}

fn read_check(expected_size: u32, res: std::io::Result<usize>) -> bool {
    match res {
        Ok(count) => {
            if count == 0 {
                return false;
            }
            if count as u32 == expected_size {
                true // read succeeded
            } else {
                log::error!("count {count} is not {expected_size}");
                false
            }
        }
        Err(_e) => {
            //log::error!("failed to get packet size: {}", e);
            false
        }
    }
}

type Payload = SmallVec<[u8; 64]>;

fn read_payload(conn: &mut local_socket::Stream, size: u32) -> Option<Payload> {
    let mut payload = Payload::new();
    payload.resize(size as usize, 0);
    if read_check(size, conn.read(&mut payload)) {
        Some(payload)
    } else {
        None
    }
}

pub struct TickParams<'a> {
    pub wvr_server: &'a mut WvrServerState,
    #[allow(dead_code)]
    pub tasks: &'a mut Vec<wayvr::TickTask>,
    pub signals: &'a SyncEventQueue<WayVRSignal>,
    pub input_state: &'a InputState,
}

pub fn gen_args_vec(input: &str) -> Vec<&str> {
    input.split_whitespace().collect()
}

pub fn gen_env_vec(input: &[String]) -> Vec<(&str, &str)> {
    input
        .iter()
        .filter_map(|e| e.as_str().split_once('='))
        .collect()
}

impl Connection {
    const fn new(conn: local_socket::Stream) -> Self {
        Self {
            conn,
            alive: true,
            auth: None,
            next_packet: None,
        }
    }

    fn kill(&mut self, reason: &str) {
        let _dont_care = send_packet(
            &mut self.conn,
            &ipc::data_encode(&PacketServer::Disconnect(packet_server::Disconnect {
                reason: String::from(reason),
            })),
        );
        self.alive = false;
    }

    fn process_handshake(&mut self, handshake: &packet_client::Handshake) -> anyhow::Result<()> {
        if self.auth.is_some() {
            anyhow::bail!("You were already authenticated");
        }

        if handshake.protocol_version != ipc::PROTOCOL_VERSION {
            anyhow::bail!(
                "Unsupported protocol version {}",
                handshake.protocol_version
            );
        }

        if handshake.magic != ipc::CONNECTION_MAGIC {
            anyhow::bail!("Invalid magic");
        }

        match handshake.client_name.len() {
            0 => anyhow::bail!("Client name is empty"),
            1..32 => {}
            _ => anyhow::bail!("Client name is too long"),
        }

        log::info!("IPC: Client \"{}\" connected.", handshake.client_name);

        self.auth = Some(AuthInfo {
            client_name: handshake.client_name.clone(),
            protocol_version: handshake.protocol_version,
        });

        // Send auth response
        send_packet(
            &mut self.conn,
            &ipc::data_encode(&PacketServer::HandshakeSuccess(
                packet_server::HandshakeSuccess {
                    runtime: String::from("wayvr-server"),
                },
            )),
        )?;

        Ok(())
    }

    fn handle_wlx_input_state(
        &mut self,
        params: &TickParams,
        serial: ipc::Serial,
    ) -> anyhow::Result<()> {
        let to_arr = |vec: &Vec3A| -> [f32; 3] { [vec.x, vec.y, vec.z] };

        send_packet(
            &mut self.conn,
            &ipc::data_encode(&PacketServer::WlxInputStateResponse(
                serial,
                packet_server::WlxInputState {
                    hmd_pos: to_arr(&params.input_state.hmd.translation),
                    left: WlxInputStatePointer {
                        pos: to_arr(&params.input_state.pointers[0].raw_pose.translation),
                    },
                    right: WlxInputStatePointer {
                        pos: to_arr(&params.input_state.pointers[0].raw_pose.translation),
                    },
                },
            )),
        )?;

        Ok(())
    }

    fn handle_wvr_window_list(
        &mut self,
        params: &mut TickParams,
        serial: ipc::Serial,
    ) -> anyhow::Result<()> {
        let mut send = |list: Option<packet_server::WvrWindowList>| -> anyhow::Result<()> {
            send_packet(
                &mut self.conn,
                &ipc::data_encode(&PacketServer::WvrWindowListResponse(serial, list)),
            )
        };

        send(Some(packet_server::WvrWindowList {
            list: params
                .wvr_server
                .wm
                .windows
                .iter()
                .map(|(handle, win)| packet_server::WvrWindow {
                    handle: wayvr::window::WindowHandle::as_packet(&handle),
                    process_handle: wayvr::process::ProcessHandle::as_packet(&win.process),
                    size_x: win.size_x,
                    size_y: win.size_y,
                    visible: win.visible,
                })
                .collect::<Vec<_>>(),
        }))
    }

    fn handle_wvr_window_set_visible(
        params: &mut TickParams,
        handle: packet_server::WvrWindowHandle,
        visible: bool,
    ) {
        if let Some(window) = params
            .wvr_server
            .wm
            .windows
            .get_mut(&wayvr::window::WindowHandle::from_packet(handle))
        {
            window.visible = visible;
        }
    }

    fn handle_wvr_process_launch(
        &mut self,
        params: &mut TickParams,
        serial: ipc::Serial,
        packet_params: packet_client::WvrProcessLaunchParams,
    ) -> anyhow::Result<()> {
        let args_vec = gen_args_vec(&packet_params.args);
        let env_vec = gen_env_vec(&packet_params.env);

        let res = params.wvr_server.spawn_process(
            &packet_params.name,
            &packet_params.exec,
            &args_vec,
            &env_vec,
            packet_params.resolution,
            packet_params.pos_mode,
            None,
            packet_params.icon.as_deref(),
            packet_params.userdata,
        );

        let res = res.map(|r| r.as_packet()).map_err(|e| e.to_string());

        send_packet(
            &mut self.conn,
            &ipc::data_encode(&PacketServer::WvrProcessLaunchResponse(serial, res)),
        )?;

        Ok(())
    }

    fn handle_wvr_process_list(
        &mut self,
        params: &TickParams,
        serial: ipc::Serial,
    ) -> anyhow::Result<()> {
        let list: Vec<packet_server::WvrProcess> = params
            .wvr_server
            .processes
            .vec
            .iter()
            .enumerate()
            .filter_map(|(idx, opt_cell)| {
                let Some(cell) = opt_cell else {
                    return None;
                };
                let process = &cell.obj;
                Some(process.to_packet(wayvr::process::ProcessHandle::new(
                    idx as u32,
                    cell.generation,
                )))
            })
            .collect();

        send_packet(
            &mut self.conn,
            &ipc::data_encode(&PacketServer::WvrProcessListResponse(
                serial,
                packet_server::WvrProcessList { list },
            )),
        )?;

        Ok(())
    }

    // This request doesn't return anything to the client
    fn handle_wvr_process_terminate(
        params: &mut TickParams,
        process_handle: packet_server::WvrProcessHandle,
    ) {
        use crate::backend::wayvr::process::KillSignal;

        let native_handle = &wayvr::process::ProcessHandle::from_packet(process_handle);
        let process = params.wvr_server.processes.get_mut(native_handle);

        let Some(process) = process else {
            return;
        };

        process.kill(KillSignal::Term);
    }

    fn handle_wvr_process_get(
        &mut self,
        params: &TickParams,
        serial: ipc::Serial,
        process_handle: packet_server::WvrProcessHandle,
    ) -> anyhow::Result<()> {
        let native_handle = &wayvr::process::ProcessHandle::from_packet(process_handle);
        let process = params
            .wvr_server
            .processes
            .get(native_handle)
            .map(|process| process.to_packet(*native_handle));

        send_packet(
            &mut self.conn,
            &ipc::data_encode(&PacketServer::WvrProcessGetResponse(serial, process)),
        )?;

        Ok(())
    }

    fn handle_wlx_device_haptics(
        params: &mut TickParams,
        device: usize,
        haptics_params: packet_client::WlxHapticsParams,
    ) {
        params.signals.send(WayVRSignal::DeviceHaptics(
            device,
            crate::backend::input::Haptics {
                duration: haptics_params.duration,
                frequency: haptics_params.frequency,
                intensity: haptics_params.intensity,
            },
        ));
    }

    fn handle_wlx_show_hide(params: &mut TickParams) {
        params.signals.send(WayVRSignal::ShowHide);
    }

    fn handle_wlx_switch_set(params: &mut TickParams, set: Option<usize>) {
        params.signals.send(WayVRSignal::SwitchSet(set));
    }

    fn handle_wlx_panel(
        params: &mut TickParams,
        custom_params: packet_client::WlxModifyPanelParams,
    ) {
        use crate::backend::task::{ModifyPanelCommand, ModifyPanelTask};

        params
            .signals
            .send(WayVRSignal::CustomTask(ModifyPanelTask {
                overlay: custom_params.overlay,
                element: custom_params.element,
                command: match custom_params.command {
                    packet_client::WlxModifyPanelCommand::SetText(text) => {
                        ModifyPanelCommand::SetText(text)
                    }
                    packet_client::WlxModifyPanelCommand::SetImage(sprite) => {
                        ModifyPanelCommand::SetImage(sprite)
                    }
                    packet_client::WlxModifyPanelCommand::SetStickyState(sticky) => {
                        ModifyPanelCommand::SetStickyState(sticky)
                    }
                    packet_client::WlxModifyPanelCommand::SetVisible(visible) => {
                        ModifyPanelCommand::SetVisible(visible)
                    }
                    packet_client::WlxModifyPanelCommand::SetColor(color) => {
                        ModifyPanelCommand::SetColor(color)
                    }
                },
            }));
    }

    // FIXME: we should probably respond an error to the client in case if wayland server feature is disabled
    //        fix this after we're done with the webkit-based wayvr-dashboard
    #[allow(unused_variables)]
    fn process_payload(&mut self, params: &mut TickParams, payload: Payload) -> anyhow::Result<()> {
        let packet: PacketClient = ipc::data_decode(&payload)?;

        if let PacketClient::Handshake(handshake) = &packet {
            self.process_handshake(handshake)?;
            return Ok(());
        }

        match packet {
            PacketClient::Handshake(_) => unreachable!(), // handled previously
            PacketClient::WlxInputState(serial) => {
                self.handle_wlx_input_state(params, serial)?;
            }
            PacketClient::WvrWindowList(serial) => {
                self.handle_wvr_window_list(params, serial)?;
            }
            PacketClient::WvrWindowSetVisible(window_handle, visible) => {
                Self::handle_wvr_window_set_visible(params, window_handle, visible);
            }
            PacketClient::WvrProcessGet(serial, process_handle) => {
                self.handle_wvr_process_get(params, serial, process_handle)?;
            }
            PacketClient::WvrProcessList(serial) => {
                self.handle_wvr_process_list(params, serial)?;
            }
            PacketClient::WvrProcessLaunch(serial, packet_params) => {
                self.handle_wvr_process_launch(params, serial, packet_params)?;
            }
            PacketClient::WvrProcessTerminate(process_handle) => {
                Self::handle_wvr_process_terminate(params, process_handle);
            }
            PacketClient::WlxDeviceHaptics(device, haptics_params) => {
                Self::handle_wlx_device_haptics(params, device, haptics_params);
            }
            PacketClient::WlxShowHide => {
                Self::handle_wlx_show_hide(params);
            }
            PacketClient::WlxSwitchSet(set) => {
                Self::handle_wlx_switch_set(params, set);
            }
            PacketClient::WlxModifyPanel(custom_params) => {
                Self::handle_wlx_panel(params, custom_params);
            }
        }

        Ok(())
    }

    fn process_check_payload(&mut self, params: &mut TickParams, payload: Payload) -> bool {
        log::debug!("payload size {}", payload.len());

        if let Err(e) = self.process_payload(params, payload) {
            log::error!("Invalid payload from the client, closing connection: {e}");
            // send also error message directly to the client before disconnecting
            self.kill(format!("{e}").as_str());
            false
        } else {
            true
        }
    }

    fn read_packet(&mut self, params: &mut TickParams) -> bool {
        if let Some(payload_size) = self.next_packet {
            let Some(payload) = read_payload(&mut self.conn, payload_size) else {
                // still failed to read payload, try in next tick
                return false;
            };

            if !self.process_check_payload(params, payload) {
                return false;
            }

            self.next_packet = None;
        }

        let mut buf_packet_header: [u8; 4] = [0; 4];
        if !read_check(4, self.conn.read(&mut buf_packet_header)) {
            return false;
        }

        let payload_size = u32::from_be_bytes(buf_packet_header[0..4].try_into().unwrap()); // 0-3 bytes (u32 size)

        let size_limit: u32 = 128 * 1024;

        if payload_size > size_limit {
            // over 128 KiB?
            log::error!(
                "Client sent a packet header with the size over {size_limit} bytes, closing connection."
            );
            self.kill("Too big packet received (over 128 KiB)");
            return false;
        }

        let Some(payload) = read_payload(&mut self.conn, payload_size) else {
            // failed to read payload, try in next tick
            self.next_packet = Some(payload_size);
            return false;
        };

        if !self.process_check_payload(params, payload) {
            return false;
        }

        true
    }

    fn tick(&mut self, params: &mut TickParams) {
        while self.read_packet(params) {}
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        log::info!("Connection closed");
    }
}

pub struct WayVRServer {
    listener: local_socket::Listener,
    connections: Vec<Connection>,
}

impl WayVRServer {
    pub fn new() -> anyhow::Result<Self> {
        let printname = "/tmp/wayvr_ipc.sock";
        let name = printname.to_ns_name::<local_socket::GenericNamespaced>()?;
        let opts = local_socket::ListenerOptions::new()
            .name(name)
            .nonblocking(local_socket::ListenerNonblockingMode::Both);
        let listener = match opts.create_sync() {
            Ok(listener) => listener,
            Err(e) => anyhow::bail!("Failed to start WayVRServer IPC listener. Reason: {e}"),
        };

        log::info!("WayVRServer IPC running at {printname}");

        Ok(Self {
            listener,
            connections: Vec::new(),
        })
    }

    fn accept_connections(&mut self) {
        let Ok(conn) = self.listener.accept() else {
            return; // No new connection or other error
        };

        self.connections.push(Connection::new(conn));
    }

    fn tick_connections(&mut self, params: &mut TickParams) {
        for c in &mut self.connections {
            c.tick(params);
        }

        // remove killed connections
        self.connections.retain(|c| c.alive);
    }

    pub fn tick(&mut self, params: &mut TickParams) {
        self.accept_connections();
        self.tick_connections(params);
    }

    pub fn broadcast(&mut self, packet: packet_server::PacketServer) {
        for connection in &mut self.connections {
            if let Err(e) = send_packet(&mut connection.conn, &ipc::data_encode(&packet)) {
                log::error!("failed to broadcast packet: {e:?}");
            }
        }
    }
}
