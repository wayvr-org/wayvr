use std::{
    error::Error,
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use xcb::{Xid, x};

use crate::{RUNNING, subsystem::clipboard::ClipboardProvider};

pub struct Provider {
    tx: Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

impl Provider {
    pub fn new() -> Result<Self, Box<dyn Error + Send + Sync + 'static>> {
        let (conn, screen_num) = xcb::Connection::connect(None)?;
        let runtime = ClipboardRuntime::new(conn, screen_num)?;

        let (tx, rx) = mpsc::channel();

        let worker = thread::spawn(move || {
            runtime.run(rx);
        });

        Ok(Self {
            tx,
            worker: Some(worker),
        })
    }
}

impl ClipboardProvider for Provider {
    fn set_clipboard_utf8(&mut self, content: &str) {
        let (ack_tx, ack_rx) = mpsc::channel();

        if self
            .tx
            .send(Command::Set(content.as_bytes().to_vec(), ack_tx))
            .is_ok()
        {
            // make sure the X server has received the ownership request before returning
            let _ = ack_rx.recv();
        }
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum Command {
    Set(Vec<u8>, Sender<()>),
    Shutdown,
}

#[derive(Clone, Copy)]
struct Atoms {
    clipboard: x::Atom,
    utf8_string: x::Atom,
    targets: x::Atom,
    text: x::Atom,
    text_plain: x::Atom,
    text_plain_utf8: x::Atom,
}

struct ClipboardRuntime {
    conn: xcb::Connection,
    window: x::Window,
    atoms: Atoms,
    content: Vec<u8>,
    owns_clipboard: bool,
}

impl ClipboardRuntime {
    fn new(
        conn: xcb::Connection,
        screen_num: i32,
    ) -> Result<Self, Box<dyn Error + Send + Sync + 'static>> {
        let atoms = Atoms {
            clipboard: intern_atom(&conn, b"CLIPBOARD")?,
            utf8_string: intern_atom(&conn, b"UTF8_STRING")?,
            targets: intern_atom(&conn, b"TARGETS")?,
            text: intern_atom(&conn, b"TEXT")?,
            text_plain: intern_atom(&conn, b"text/plain")?,
            text_plain_utf8: intern_atom(&conn, b"text/plain;charset=utf-8")?,
        };

        let setup = conn.get_setup();
        let screen = setup.roots().nth(screen_num as usize).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "X11 screen not found")
        })?;

        let window: x::Window = conn.generate_id();

        conn.send_and_check_request(&x::CreateWindow {
            depth: x::COPY_FROM_PARENT as u8,
            wid: window,
            parent: screen.root(),
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            border_width: 0,
            class: x::WindowClass::InputOutput,
            visual: screen.root_visual(),
            value_list: &[],
        })?;

        conn.flush()?;

        Ok(Self {
            conn,
            window,
            atoms,
            content: Vec::new(),
            owns_clipboard: false,
        })
    }

    fn run(mut self, rx: Receiver<Command>) {
        while RUNNING.load(Ordering::Relaxed) {
            self.drain_x_events();

            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(command) => {
                    if !self.handle_command(command) {
                        break;
                    }

                    while let Ok(command) = rx.try_recv() {
                        if !self.handle_command(command) {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn handle_command(&mut self, command: Command) -> bool {
        match command {
            Command::Set(content, ack) => {
                self.content = content;
                self.become_clipboard_owner();
                let _ = ack.send(());
                true
            }
            Command::Shutdown => {
                self.release_clipboard_if_owned();
                false
            }
        }
    }

    fn become_clipboard_owner(&mut self) {
        self.conn.send_request(&x::SetSelectionOwner {
            owner: self.window,
            selection: self.atoms.clipboard,
            time: x::CURRENT_TIME,
        });

        self.owns_clipboard = true;
        let _ = self.conn.flush();
    }

    fn release_clipboard_if_owned(&mut self) {
        if !self.owns_clipboard {
            return;
        }

        self.conn.send_request(&x::SetSelectionOwner {
            owner: x::Window::none(),
            selection: self.atoms.clipboard,
            time: x::CURRENT_TIME,
        });

        self.owns_clipboard = false;
        let _ = self.conn.flush();
    }

    #[allow(clippy::match_same_arms)]
    fn drain_x_events(&mut self) {
        loop {
            match self.conn.poll_for_event() {
                Ok(Some(event)) => self.handle_x_event(event),
                Ok(None) => break,
                Err(xcb::Error::Protocol(_)) => { /* continue */ }
                Err(xcb::Error::Connection(_)) => break,
            }
        }
    }

    fn handle_x_event(&mut self, event: xcb::Event) {
        match event {
            xcb::Event::X(x::Event::SelectionRequest(req)) => {
                self.handle_selection_request(&req);
            }
            xcb::Event::X(x::Event::SelectionClear(ev))
                if ev.selection() == self.atoms.clipboard =>
            {
                self.owns_clipboard = false;
            }
            _ => {}
        }
    }

    fn handle_selection_request(&self, req: &x::SelectionRequestEvent) {
        if !self.owns_clipboard || req.selection() != self.atoms.clipboard {
            self.send_selection_notify(req, x::Atom::none());
            return;
        }

        let property = if req.property().is_none() {
            // compatibility with old clients that use XCB_NONE for property
            req.target()
        } else {
            req.property()
        };

        let ok = if req.target() == self.atoms.targets {
            self.write_targets(req.requestor(), property)
        } else if let Some(type_atom) = self.type_for_text_target(req.target()) {
            self.write_text(req.requestor(), property, type_atom)
        } else {
            false
        };

        self.send_selection_notify(req, if ok { property } else { x::Atom::none() });
    }

    fn supported_targets(&self) -> Vec<x::Atom> {
        let mut targets = vec![
            self.atoms.targets,
            self.atoms.utf8_string,
            self.atoms.text_plain_utf8,
            self.atoms.text_plain,
        ];

        // STRING/TEXT are not UTF-8 targets. Only advertise them when the
        // content is representable as plain ASCII.
        if self.content.is_ascii() {
            targets.push(self.atoms.text);
            targets.push(x::ATOM_STRING);
        }

        targets
    }

    fn type_for_text_target(&self, target: x::Atom) -> Option<x::Atom> {
        if target == self.atoms.utf8_string
            || target == self.atoms.text_plain_utf8
            || target == self.atoms.text_plain
            || (self.content.is_ascii() && (target == self.atoms.text || target == x::ATOM_STRING))
        {
            Some(target)
        } else {
            None
        }
    }

    fn write_targets(&self, requestor: x::Window, property: x::Atom) -> bool {
        let targets = self.supported_targets();

        self.conn
            .send_and_check_request(&x::ChangeProperty {
                mode: x::PropMode::Replace,
                window: requestor,
                property,
                r#type: x::ATOM_ATOM,
                data: targets.as_slice(),
            })
            .is_ok()
    }

    fn write_text(&self, requestor: x::Window, property: x::Atom, type_atom: x::Atom) -> bool {
        self.conn
            .send_and_check_request(&x::ChangeProperty {
                mode: x::PropMode::Replace,
                window: requestor,
                property,
                r#type: type_atom,
                data: self.content.as_slice(),
            })
            .is_ok()
    }

    fn send_selection_notify(&self, req: &x::SelectionRequestEvent, property: x::Atom) {
        let event = x::SelectionNotifyEvent::new(
            req.time(),
            req.requestor(),
            req.selection(),
            req.target(),
            property,
        );

        let _ = self.conn.send_and_check_request(&x::SendEvent {
            propagate: false,
            destination: x::SendEventDest::Window(req.requestor()),
            event_mask: x::EventMask::NO_EVENT,
            event: &event,
        });

        let _ = self.conn.flush();
    }
}

fn intern_atom(conn: &xcb::Connection, name: &[u8]) -> xcb::Result<x::Atom> {
    let cookie = conn.send_request(&x::InternAtom {
        only_if_exists: false,
        name,
    });

    Ok(conn.wait_for_reply(cookie)?.atom())
}
