use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use idmap::IdMap;
use log::debug;

use smithay_client_toolkit::reexports::{
    protocols::xdg::xdg_output::zv1::client::{
        zxdg_output_manager_v1::ZxdgOutputManagerV1,
        zxdg_output_v1::{self, ZxdgOutputV1},
    },
    protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    backend::WaylandError,
    globals::{GlobalList, GlobalListContents, registry_queue_init},
    protocol::{
        wl_output::{self, Transform, WlOutput},
        wl_registry::{self, WlRegistry},
        wl_seat::WlSeat,
        wl_shm::WlShm,
    },
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};

use crate::frame;

pub enum OutputChangeEvent {
    /// New output has been created.
    Create(u32),
    /// Logical position or size has changed, but no changes required in terms of rendering.
    Logical(u32),
    /// Resolution or transform has changed, textures need to be recreated.
    Physical(u32),
    /// Output has been destroyed.
    Destroy(u32),
}

pub struct WlxOutput {
    pub wl_output: WlOutput,
    pub id: u32,
    pub name: Arc<str>,
    pub make: Arc<str>,
    pub model: Arc<str>,
    pub size: (i32, i32),
    pub logical_pos: (i32, i32),
    pub logical_size: (i32, i32),
    pub transform: frame::Transform,
    done: bool,
}

pub struct WlxClient {
    pub connection: Arc<Connection>,
    pub xdg_output_mgr: ZxdgOutputManagerV1,
    pub maybe_wlr_screencopy_mgr: Option<ZwlrScreencopyManagerV1>,
    pub maybe_zwp_linux_dmabuf: Option<ZwpLinuxDmabufV1>,
    pub wl_seat: WlSeat,
    pub wl_shm: WlShm,
    pub outputs: IdMap<u32, WlxOutput>,
    pub queue: Arc<Mutex<EventQueue<Self>>>,
    pub globals: GlobalList,
    pub queue_handle: QueueHandle<Self>,
    default_output_name: Arc<str>,
    events: VecDeque<OutputChangeEvent>,
}

impl WlxClient {
    pub fn new() -> Option<Self> {
        let connection = Connection::connect_to_env()
            .inspect_err(|e| log::info!("Wayland connection: {e:?}"))
            .ok()?;
        let (globals, queue) = registry_queue_init::<Self>(&connection)
            .inspect_err(|e| log::info!("Wayland queue init: {e:?}"))
            .ok()?;
        let qh = queue.handle();

        let mut state = Self {
            connection: Arc::new(connection),
            xdg_output_mgr: globals
                .bind(&qh, 2..=3, ())
                .expect(ZxdgOutputManagerV1::interface().name),
            wl_seat: globals
                .bind(&qh, 4..=9, ())
                .expect(WlSeat::interface().name),
            wl_shm: globals.bind(&qh, 1..=1, ()).expect(WlShm::interface().name),
            maybe_wlr_screencopy_mgr: globals.bind(&qh, 3..=3, ()).ok(),
            maybe_zwp_linux_dmabuf: globals.bind(&qh, 4..=4, ()).ok(),
            outputs: IdMap::new(),
            queue: Arc::new(Mutex::new(queue)),
            globals,
            queue_handle: qh,
            default_output_name: "Unknown".into(),
            events: VecDeque::new(),
        };

        for o in state.globals.contents().clone_list().iter() {
            if o.interface == WlOutput::interface().name {
                state.add_output(o.name, o.version);
            }
        }

        state.dispatch();

        Some(state)
    }

    fn add_output(&mut self, name: u32, version: u32) {
        let wl_output: WlOutput =
            self.globals
                .registry()
                .bind(name, version, &self.queue_handle, name);
        self.xdg_output_mgr
            .get_xdg_output(&wl_output, &self.queue_handle, name);
        let output = WlxOutput {
            wl_output,
            id: name,
            name: self.default_output_name.clone(),
            make: self.default_output_name.clone(),
            model: self.default_output_name.clone(),
            size: (0, 0),
            logical_pos: (0, 0),
            logical_size: (0, 0),
            transform: frame::Transform::Normal,
            done: false,
        };

        self.outputs.insert(name, output);
    }

    pub fn get_desktop_origin(&self) -> (i32, i32) {
        let mut origin = (i32::MAX, i32::MAX);
        for (_, output) in self.outputs.iter() {
            origin.0 = origin.0.min(output.logical_pos.0);
            origin.1 = origin.1.min(output.logical_pos.1);
        }
        origin
    }

    /// Get the logical width and height of the desktop.
    pub fn get_desktop_extent(&self) -> (i32, i32) {
        let mut extent = (0, 0);
        for (_, output) in self.outputs.iter() {
            extent.0 = extent.0.max(output.logical_pos.0 + output.logical_size.0);
            extent.1 = extent.1.max(output.logical_pos.1 + output.logical_size.1);
        }
        let origin = self.get_desktop_origin();
        (extent.0 - origin.0, extent.1 - origin.1)
    }

    pub fn iter_events(&mut self) -> impl Iterator<Item = OutputChangeEvent> + '_ {
        self.events.drain(..)
    }

    /// Dispatch pending events and block until finished.
    pub fn dispatch(&mut self) {
        if let Ok(mut queue_mut) = self.queue.clone().lock() {
            let _ = queue_mut.blocking_dispatch(self);
        }
    }

    /// Dispatch pending events without blocking.
    pub fn dispatch_pending(&mut self) {
        if let Ok(mut queue_mut) = self.queue.clone().lock() {
            if let Some(reader) = queue_mut.prepare_read() {
                match reader.read() {
                    Ok(n) => match queue_mut.dispatch_pending(self) {
                        Ok(n2) => {
                            log::debug!("Read {n}, dispatched {n2} pending events");
                        }
                        Err(err) => {
                            log::warn!("Error while dispatching {n} pending events: {err:?}");
                        }
                    },
                    Err(err) => {
                        if let WaylandError::Io(ref e) = err
                            && e.kind() == std::io::ErrorKind::WouldBlock
                        {
                            return;
                        }
                        log::warn!("Error while reading from event queue: {err:?}");
                    }
                }
            } else {
                let _ = queue_mut.dispatch_pending(self);
            }
        }
    }
}

fn wl_transform_to_frame_transform(transform: Transform) -> frame::Transform {
    match transform {
        Transform::Normal => frame::Transform::Normal,
        Transform::_90 => frame::Transform::Rotated90,
        Transform::_180 => frame::Transform::Rotated180,
        Transform::_270 => frame::Transform::Rotated270,
        Transform::Flipped => frame::Transform::Flipped,
        Transform::Flipped90 => frame::Transform::Flipped90,
        Transform::Flipped180 => frame::Transform::Flipped180,
        Transform::Flipped270 => frame::Transform::Flipped270,
        _ => frame::Transform::Undefined,
    }
}

impl Dispatch<ZxdgOutputV1, u32> for WlxClient {
    fn event(
        state: &mut Self,
        _proxy: &ZxdgOutputV1,
        event: <ZxdgOutputV1 as Proxy>::Event,
        data: &u32,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        fn finalize_output(output: &mut WlxOutput) {
            if output.logical_size.0 < 0 {
                output.logical_pos.0 += output.logical_size.0;
                output.logical_size.0 *= -1;
            }
            if output.logical_size.1 < 0 {
                output.logical_pos.1 += output.logical_size.1;
                output.logical_size.1 *= -1;
            }
            if !output.done {
                output.done = true;
                debug!(
                    "Discovered WlOutput {}; Size: {:?}; Logical Size: {:?}; Pos: {:?}",
                    output.name, output.size, output.logical_size, output.logical_pos
                );
            }
        }
        match event {
            zxdg_output_v1::Event::Name { name } => {
                if let Some(output) = state.outputs.get_mut(*data) {
                    output.name = name.into();
                }
            }
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                if let Some(output) = state.outputs.get_mut(*data) {
                    output.logical_pos = (x, y);
                    let was_done = output.done;
                    if output.logical_size != (0, 0) {
                        finalize_output(output);
                    }
                    if was_done {
                        log::info!(
                            "{}: Logical pos changed to {:?}",
                            output.name,
                            output.logical_pos,
                        );
                        state.events.push_back(OutputChangeEvent::Logical(*data));
                    } else {
                        state.events.push_back(OutputChangeEvent::Create(*data));
                    }
                }
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                if let Some(output) = state.outputs.get_mut(*data) {
                    output.logical_size = (width, height);
                    let was_done = output.done;
                    if output.logical_pos != (0, 0) {
                        finalize_output(output);
                    }
                    if was_done {
                        log::info!(
                            "{}: Logical size changed to {:?}",
                            output.name,
                            output.logical_size,
                        );
                        state.events.push_back(OutputChangeEvent::Logical(*data));
                    } else {
                        state.events.push_back(OutputChangeEvent::Create(*data));
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for WlxClient {
    fn event(
        state: &mut Self,
        _proxy: &WlOutput,
        event: <WlOutput as Proxy>::Event,
        data: &u32,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Mode {
                width,
                height,
                flags,
                ..
            } => {
                if !flags
                    .into_result()
                    .is_ok_and(|f| f.intersects(wl_output::Mode::Current))
                {
                    // https://github.com/galister/wlx-capture/issues/5
                    return;
                }

                if let Some(output) = state.outputs.get_mut(*data) {
                    output.size = (width, height);
                    if output.done {
                        log::info!(
                            "{}: Resolution changed {:?} -> {:?}",
                            output.name,
                            output.size,
                            (width, height)
                        );
                        state.events.push_back(OutputChangeEvent::Physical(*data));
                    }
                }
            }
            wl_output::Event::Geometry {
                make,
                model,
                transform,
                ..
            } => {
                if let Some(output) = state.outputs.get_mut(*data) {
                    let transform = transform.into_result().unwrap_or(Transform::Normal);
                    let old_transform = output.transform;
                    output.transform = wl_transform_to_frame_transform(transform);
                    if output.done && old_transform != output.transform {
                        log::info!(
                            "{}: Transform changed {:?} -> {:?}",
                            output.name,
                            output.transform,
                            transform
                        );
                        state.events.push_back(OutputChangeEvent::Physical(*data));
                        state.events.push_back(OutputChangeEvent::Logical(*data));
                    }
                    output.make = make.into();
                    output.model = model.into();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for WlxClient {
    fn event(
        state: &mut Self,
        _proxy: &WlRegistry,
        event: <WlRegistry as Proxy>::Event,
        _data: &GlobalListContents,
        conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                if interface == WlOutput::interface().name {
                    state.add_output(name, version);
                    let _ = conn.roundtrip();
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(output) = state.outputs.remove(name) {
                    log::info!("{}: Device removed", output.name);
                    state.events.push_back(OutputChangeEvent::Destroy(name));
                }
            }
            _ => {}
        }
    }
}

// Plumbing below

impl Dispatch<ZxdgOutputManagerV1, ()> for WlxClient {
    fn event(
        _state: &mut Self,
        _proxy: &ZxdgOutputManagerV1,
        _event: <ZxdgOutputManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for WlxClient {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrScreencopyManagerV1,
        _event: <ZwlrScreencopyManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for WlxClient {
    fn event(
        _state: &mut Self,
        _proxy: &WlSeat,
        _event: <WlSeat as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShm, ()> for WlxClient {
    fn event(
        _state: &mut Self,
        _proxy: &WlShm,
        _event: <WlShm as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpLinuxDmabufV1, ()> for WlxClient {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpLinuxDmabufV1,
        _event: <ZwpLinuxDmabufV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpLinuxBufferParamsV1, ()> for WlxClient {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpLinuxBufferParamsV1,
        _event: <ZwpLinuxBufferParamsV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}
