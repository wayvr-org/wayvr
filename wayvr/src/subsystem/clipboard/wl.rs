use std::{
    fs::File,
    io::Write,
    sync::mpsc::{self, SyncSender},
    thread::{self, JoinHandle},
};

use anyhow::Context;
use calloop::{
    EventLoop,
    channel::{Channel, Event as ChannelEvent, Sender, channel},
};
use calloop_wayland_source::WaylandSource;
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_registry,
        wl_seat::{self, WlSeat},
    },
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::{self, ExtDataControlManagerV1},
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};

use crate::subsystem::clipboard::ClipboardProvider;

const TEXT_MIME_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain;charset=UTF-8",
    "text/plain",
    "UTF8_STRING",
    "TEXT",
    "STRING",
];

pub struct Provider {
    tx: Sender<Command>,
    _thread: JoinHandle<()>,
}

enum Command {
    SetText(String),
    Shutdown,
}

struct ActiveSource {
    source: ExtDataControlSourceV1,
    bytes: Vec<u8>,
}

struct State {
    conn: Connection,
    qh: QueueHandle<State>,
    manager: ExtDataControlManagerV1,
    device: ExtDataControlDeviceV1,
    active_sources: Vec<ActiveSource>,
    ignored_offers: Vec<ExtDataControlOfferV1>,
    stop: bool,
}

impl Provider {
    pub fn new() -> anyhow::Result<Self> {
        let (tx, rx) = channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);

        let thread = thread::Builder::new()
            .name("wayland-clipboard-ext-data-control".to_owned())
            .spawn(move || run_worker(rx, ready_tx))
            .context("failed to spawn Wayland clipboard worker thread")?;

        match ready_rx
            .recv()
            .context("Wayland clipboard worker exited before reporting readiness")?
        {
            Ok(()) => Ok(Self {
                tx,
                _thread: thread,
            }),
            Err(err) => anyhow::bail!("{err}"),
        }
    }
}

impl ClipboardProvider for Provider {
    fn set_clipboard_utf8(&mut self, content: &str) {
        let _ = self.tx.send(Command::SetText(content.to_owned()));
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
    }
}

fn run_worker(rx: Channel<Command>, ready: SyncSender<Result<(), String>>) {
    macro_rules! init_or_report {
        ($expr:expr, $context:literal) => {
            match $expr {
                Ok(value) => value,
                Err(err) => {
                    let _ = ready.send(Err(format!("{}: {}", $context, err)));
                    return;
                }
            }
        };
    }

    let conn = init_or_report!(
        Connection::connect_to_env(),
        "connect to Wayland compositor"
    );

    let (globals, event_queue) =
        init_or_report!(registry_queue_init::<State>(&conn), "read Wayland globals");

    let qh = event_queue.handle();

    let mut event_loop =
        init_or_report!(EventLoop::<State>::try_new(), "create calloop event loop");

    init_or_report!(
        WaylandSource::new(conn.clone(), event_queue).insert(event_loop.handle()),
        "attach Wayland queue to event loop"
    );

    let manager: ExtDataControlManagerV1 = init_or_report!(
        globals.bind(&qh, 1..=1, ()),
        "bind ext_data_control_manager_v1"
    );

    let seat: WlSeat = init_or_report!(globals.bind(&qh, 1..=9, ()), "bind wl_seat");

    let device = manager.get_data_device(&seat, &qh, ());

    let mut state = State {
        conn,
        qh,
        manager,
        device,
        active_sources: Vec::new(),
        ignored_offers: Vec::new(),
        stop: false,
    };

    init_or_report!(
        event_loop
            .handle()
            .insert_source(rx, |event, _metadata, state| match event {
                ChannelEvent::Msg(Command::SetText(text)) => {
                    state.set_clipboard_text(text);
                }
                ChannelEvent::Msg(Command::Shutdown) | ChannelEvent::Closed => {
                    state.stop = true;
                }
            }),
        "attach clipboard command channel to event loop"
    );

    if let Err(err) = state.conn.flush() {
        let _ = ready.send(Err(format!("flush initial Wayland requests: {err}")));
        return;
    }

    let _ = ready.send(Ok(()));

    while !state.stop {
        if let Err(err) = event_loop.dispatch(None, &mut state) {
            eprintln!("Wayland clipboard event loop failed: {err}");
            break;
        }

        if let Err(err) = state.conn.flush() {
            eprintln!("Wayland clipboard flush failed: {err}");
            break;
        }
    }
}

impl State {
    fn set_clipboard_text(&mut self, content: String) {
        let source = self.manager.create_data_source(&self.qh, ());

        for mime_type in TEXT_MIME_TYPES {
            source.offer((*mime_type).to_owned());
        }

        self.device.set_selection(Some(&source));

        let previous_sources = std::mem::take(&mut self.active_sources);

        self.active_sources.push(ActiveSource {
            source,
            bytes: content.into_bytes(),
        });

        for old in previous_sources {
            old.source.destroy();
        }

        let _ = self.conn.flush();
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _state: &mut Self,
        _seat: &WlSeat,
        _event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlManagerV1, ()> for State {
    fn event(
        _state: &mut Self,
        _manager: &ExtDataControlManagerV1,
        _event: ext_data_control_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        device: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                state.ignored_offers.push(id);
            }
            ext_data_control_device_v1::Event::Selection { id }
            | ext_data_control_device_v1::Event::PrimarySelection { id } => {
                if let Some(offer) = id {
                    if let Some(pos) = state
                        .ignored_offers
                        .iter()
                        .position(|stored| *stored == offer)
                    {
                        state.ignored_offers.swap_remove(pos);
                    }

                    offer.destroy();
                }
            }
            ext_data_control_device_v1::Event::Finished => {
                device.destroy();
                state.stop = true;
            }
            _ => {}
        }
    }

    fn event_created_child(
        opcode: u16,
        qh: &QueueHandle<Self>,
    ) -> std::sync::Arc<dyn wayland_client::backend::ObjectData> {
        if opcode == ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE {
            qh.make_data::<ExtDataControlOfferV1, _>(())
        } else {
            unreachable!("unknown ext_data_control_device_v1 child event opcode: {opcode}")
        }
    }
}

impl Dispatch<ExtDataControlOfferV1, ()> for State {
    fn event(
        _state: &mut Self,
        _offer: &ExtDataControlOfferV1,
        _event: ext_data_control_offer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlSourceV1, ()> for State {
    fn event(
        state: &mut Self,
        source: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                if !TEXT_MIME_TYPES.contains(&mime_type.as_str()) {
                    return;
                }

                let Some(active) = state
                    .active_sources
                    .iter()
                    .find(|active| active.source == *source)
                else {
                    return;
                };

                let mut file = File::from(fd);

                if let Err(err) = file.write_all(&active.bytes) {
                    eprintln!("failed to write clipboard data to Wayland fd: {err}");
                    return;
                }

                let _ = file.flush();
            }
            ext_data_control_source_v1::Event::Cancelled => {
                if let Some(pos) = state
                    .active_sources
                    .iter()
                    .position(|active| active.source == *source)
                {
                    let active = state.active_sources.swap_remove(pos);
                    active.source.destroy();
                }
            }
            _ => {}
        }
    }
}
