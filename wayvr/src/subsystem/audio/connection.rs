use super::{AudioEvent, Command, SharedSnapshot};
use crate::subsystem::audio::types::{APCInner, AudioPipewireOwner, Output};
use pipewire::core::{CoreRc, Listener, PW_ID_CORE};
use pipewire::registry::RegistryRc;
use pipewire::{self as pw, context::ContextRc, main_loop::MainLoopRc};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::{sync::mpsc::Sender, time::Duration};

pub(super) struct AudioServiceLaunch {
    pub commands: pw::channel::Receiver<Command>,
    pub snapshot: SharedSnapshot,
    pub events: Sender<AudioEvent>,
}

impl AudioPipewireOwner {
    pub(super) fn new(output: Output, registry: RegistryRc) -> Self {
        let new = Self(Rc::new_cyclic(|weak| {
            RefCell::new(APCInner {
                weak: weak.clone(),
                output,
                registry,
                global_listener: None,
                pending_default_sink: None,
                max_serial: 0,
                metadata_id: None,
                metadata_listener: None,
                metadata: None,
                devices: vec![],
                nodes: vec![],
                default_source: None,
                default_sink: None,
            })
        }));
        new.create_listeners();
        new
    }

    fn create_listeners(&self) {
        let mut state = self.0.borrow_mut();
        let listener = state
            .registry
            .add_listener_local()
            .global({
                let state = state.new_outer();
                move |obj| state.run(|s| s.on_global(obj))
            })
            .global_remove({
                let state = state.new_outer();
                move |id| {
                    state.run(|s| {
                        s.nodes.retain(|n| n.id != id);
                        s.devices.retain(|n| n.id != id);
                        if s.metadata_id == Some(id) {
                            s.metadata_listener = None;
                            s.metadata = None;
                            s.metadata_id = None;
                        }
                        s.publish_snapshot();
                    });
                }
            })
            .register();
        state.global_listener = Some(listener);
    }
}

struct Session {
    owner: AudioPipewireOwner,
    _listener: Listener,
    _core: CoreRc,
    disconnected: Rc<Cell<bool>>,
}
impl Session {
    fn connect(context: &ContextRc, output: Output) -> anyhow::Result<Session> {
        let core = context.connect_rc(None)?;
        let registry = core.get_registry_rc()?;
        let state = AudioPipewireOwner::new(output, registry);

        let disconnected: Rc<Cell<bool>> = Default::default();
        let core_listener = core
            .add_listener_local()
            .error({
                let disconnected = disconnected.clone();
                move |id, _seq, res, msg| {
                    if id == PW_ID_CORE && res == -libc::EPIPE {
                        log::warn!("PipeWire connection lost: {msg}");
                        disconnected.set(true);
                    }
                }
            })
            .register();
        Ok(Self {
            owner: state,
            _listener: core_listener,
            _core: core,
            disconnected,
        })
    }

    fn is_alive(&self) -> bool {
        !self.disconnected.get()
    }
}

impl AudioServiceLaunch {
    pub fn launch(self) {
        if let Err(e) = self.run() {
            log::error!("Audio service stopped: {e:?}");
        }
    }

    fn run(self) -> anyhow::Result<()> {
        pw::init();

        let main_loop = MainLoopRc::new(None)?;
        let context = ContextRc::new(&main_loop, None)?;
        let output = Output {
            snapshot: self.snapshot,
            events: self.events,
        };

        let session: Rc<RefCell<Option<Session>>> = Default::default();

        let _handler = self.commands.attach(main_loop.loop_(), {
            let main_loop = main_loop.downgrade();
            let session = session.clone();
            move |cmd| match cmd {
                Command::Task(task) => {
                    if let Some(s) = &*session.borrow() {
                        s.owner.0.borrow_mut().handle_task(task);
                    }
                }
                Command::Stop => {
                    if let Some(main_loop) = main_loop.upgrade() {
                        main_loop.quit();
                    }
                }
            }
        });

        let timer = main_loop.loop_().add_timer(move |_| {
            let dead = session.borrow().as_ref().is_none_or(|s| !s.is_alive());
            if dead {
                if session.borrow().is_some() {
                    *session.borrow_mut() = None;
                    output.publish_empty();
                }
                *session.borrow_mut() = Session::connect(&context, output.clone()).ok();
            }
        });
        timer.update_timer(Some(Duration::from_millis(1)), Some(Duration::from_secs(5)));

        main_loop.run();
        Ok(())
    }
}
