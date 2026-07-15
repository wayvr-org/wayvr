use anyhow::Context;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::{BufferType, buffer_type};
use smithay::desktop::{
    PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, PopupUngrabStrategy,
    find_popup_root_surface, get_popup_toplevel_coords,
};
use smithay::input::pointer::{CursorImageStatus, Focus};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::rustix::fs::{OFlags, fcntl_setfl};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_callback, wl_output, wl_seat};
use smithay::reexports::wayland_server::{self, DisplayHandle};
use smithay::utils::{Logical, Rectangle, Serial, Size};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{self, BufferAssignment, SurfaceAttributes, send_surface_state};
use smithay::wayland::dmabuf::{
    DmabufFeedback, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier, get_dmabuf,
};
use smithay::wayland::fractional_scale::with_fractional_scale;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
    set_data_device_focus, set_data_device_selection,
};
use smithay::wayland::selection::{self, SelectionHandler};
use smithay::wayland::selection::{
    ext_data_control as selection_ext,
    primary_selection::{PrimarySelectionHandler, PrimarySelectionState, set_primary_focus},
    wlr_data_control as selection_wlr,
};
use smithay::wayland::shell::kde::decoration::{KdeDecorationHandler, KdeDecorationState};
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::wayland::shm::{ShmHandler, ShmState, with_buffer_contents};
use smithay::wayland::single_pixel_buffer::get_single_pixel_buffer;
use smithay::wayland::viewporter::ViewporterState;
use smithay::{
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_ext_data_control, delegate_kde_decoration, delegate_output,
    delegate_primary_selection, delegate_relative_pointer, delegate_seat, delegate_shm,
    delegate_single_pixel_buffer, delegate_viewporter, delegate_xdg_decoration, delegate_xdg_shell,
};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};
use wayland_server::Client;
use wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use wayland_server::protocol::wl_surface::WlSurface;

use crate::backend::wayvr::image_importer::ImageImporter;
use crate::backend::wayvr::{SurfaceBufWithImage, WAYVR_SCREEN_RES, time};
use crate::graphics::ExtentExt;
use crate::ipc::event_queue::SyncEventQueue;

use super::WayVRTask;

pub struct Application {
    pub output: Output,
    pub image_importer: ImageImporter,
    pub dmabuf_state: (DmabufState, DmabufGlobal, Option<DmabufFeedback>),
    pub compositor: compositor::CompositorState,
    pub xdg_shell: XdgShellState,
    pub seat: Seat<Application>,
    pub seat_state: SeatState<Application>,
    pub shm: ShmState,
    pub data_device: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub ext_data_control_state: selection_ext::DataControlState,
    pub wlr_data_control_state: selection_wlr::DataControlState,
    #[allow(dead_code)]
    pub xdg_decoration_state: XdgDecorationState,
    pub kde_decoration_state: KdeDecorationState,
    #[allow(dead_code)]
    pub relative_pointer_state: RelativePointerManagerState,
    pub wayvr_tasks: SyncEventQueue<WayVRTask>,
    pub popup_manager: PopupManager,
    #[allow(dead_code)]
    pub viewporter: ViewporterState,
    pub display_handle: DisplayHandle,
    pub redraw_requests: HashSet<ObjectId>,
    pub pending_frame_callbacks: HashMap<ObjectId, Vec<wl_callback::WlCallback>>,
    pub cursor_image: CursorImageStatus,
}

impl Application {
    pub fn cleanup(&mut self) {
        self.image_importer.cleanup();
        self.output.cleanup();
    }

    pub fn set_clipboard_text(&mut self, content: &str) {
        let mime_types = vec![
            "text/plain;charset=utf-8".to_string(),
            "text/plain".to_string(),
        ];
        let user_data = Arc::from(content.as_bytes());
        set_data_device_selection(&self.display_handle, &self.seat, mime_types, user_data);
    }

    fn popups_commit(&mut self, surface: &WlSurface) {
        self.popup_manager.commit(surface);

        if let Some(popup) = self.popup_manager.find_popup(surface) {
            match popup {
                PopupKind::Xdg(ref popup) => {
                    if !popup.is_initial_configure_sent() {
                        smithay::wayland::compositor::with_states(surface, |states| {
                            send_surface_state(
                                surface,
                                states,
                                1,
                                smithay::utils::Transform::Normal,
                            );
                            with_fractional_scale(states, |fractional| {
                                fractional.set_preferred_scale(1.0);
                            });
                        });
                        popup.send_configure().expect("initial configure failed");
                    }
                }
                PopupKind::InputMethod(_) => {
                    // TODO?
                }
            }
        }
    }

    fn send_initial_surface_state(&self, surface: &WlSurface) {
        self.output.enter(surface);

        smithay::wayland::compositor::with_states(surface, |states| {
            send_surface_state(surface, states, 1, smithay::utils::Transform::Normal);

            with_fractional_scale(states, |fractional| {
                fractional.set_preferred_scale(1.0);
            });
        });
    }

    pub fn send_frame_callbacks_for_surface_id(&mut self, surface_id: &ObjectId) {
        let Some(callbacks) = self.pending_frame_callbacks.remove(surface_id) else {
            return;
        };

        let t = time::get_millis() as u32;
        for cb in callbacks {
            cb.done(t);
        }
    }

    pub fn has_pending_frame_callbacks(
        &self,
        surface_id: &wayland_server::backend::ObjectId,
    ) -> bool {
        self.pending_frame_callbacks
            .get(surface_id)
            .is_some_and(|v| !v.is_empty())
    }

    pub fn take_redraw_request(&mut self, surface_id: &ObjectId) -> bool {
        self.redraw_requests.remove(surface_id)
    }

    pub fn output_logical_size(&self) -> Size<i32, Logical> {
        self.output
            .current_mode()
            .map(|mode| Size::new(mode.size.w, mode.size.h))
            .unwrap_or_else(|| Size::new(WAYVR_SCREEN_RES[0], WAYVR_SCREEN_RES[1]))
    }

    fn surface_logical_size(surface: &WlSurface) -> Option<Size<i32, Logical>> {
        smithay::wayland::compositor::with_states(surface, |states| {
            SurfaceBufWithImage::get_from_surface(states).map(|buf| {
                let extent = buf.image.extent_u32arr();
                let scale = buf.scale.max(1) as u32;

                Size::new((extent[0] / scale) as i32, (extent[1] / scale) as i32)
            })
        })
    }

    fn constrain_popup(&self, popup: &PopupSurface) {
        let popup_kind = PopupKind::Xdg(popup.clone());

        let Ok(root_surface) = find_popup_root_surface(&popup_kind) else {
            log::warn!(
                "constrain_popup: could not find popup root surface {:?}",
                popup.wl_surface().id()
            );
            return;
        };

        let root_size =
            Self::surface_logical_size(&root_surface).unwrap_or_else(|| self.output_logical_size());

        let mut target = Rectangle::from_size(root_size);
        target.loc -= get_popup_toplevel_coords(&popup_kind);

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

impl compositor::CompositorHandler for Application {
    fn compositor_state(&mut self) -> &mut compositor::CompositorState {
        &mut self.compositor
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a Client,
    ) -> &'a compositor::CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    #[allow(clippy::significant_drop_tightening)]
    fn commit(&mut self, surface: &WlSurface) {
        self.popups_commit(surface);

        smithay::wayland::compositor::with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            let attrs = guard.current();

            match attrs.buffer.take() {
                Some(BufferAssignment::NewBuffer(buffer)) => {
                    match buffer_type(&buffer) {
                        Some(BufferType::Dma) => {
                            let dmabuf = get_dmabuf(&buffer).unwrap(); // always Ok due to buffer_type

                            if let Ok(image) =
                                self.image_importer.get_or_import_dmabuf(dmabuf.clone())
                            {
                                let sbwi = SurfaceBufWithImage {
                                    image,
                                    transform: wl_transform_to_frame_transform(
                                        attrs.buffer_transform,
                                    ),
                                    scale: attrs.buffer_scale,
                                    dmabuf: true,
                                };

                                if let Some(old_buffer) =
                                    sbwi.apply_to_surface(states, Some(buffer.clone()))
                                {
                                    old_buffer.release();
                                }
                            } else {
                                buffer.release();
                            }
                        }
                        Some(BufferType::Shm) => {
                            let _ = with_buffer_contents(&buffer, |data, size, buf| {
                                if let Ok(image) = self
                                    .image_importer
                                    .import_shm(data, size, buf)
                                    .inspect_err(|e| {
                                        log::warn!("wayland_server failed to import SHM: {e:?}");
                                    })
                                {
                                    let sbwi = SurfaceBufWithImage {
                                        image,
                                        transform: wl_transform_to_frame_transform(
                                            attrs.buffer_transform,
                                        ),
                                        scale: attrs.buffer_scale,
                                        dmabuf: false,
                                    };
                                    sbwi.apply_to_surface(states, None);
                                }
                            });
                            buffer.release();
                        }
                        Some(BufferType::SinglePixel) => {
                            let spb = get_single_pixel_buffer(&buffer).unwrap(); // always Ok
                            if let Ok(image) =
                                self.image_importer.import_spb(spb).inspect_err(|e| {
                                    log::warn!("wayland_server failed to import SPB: {e:?}");
                                })
                            {
                                let sbwi = SurfaceBufWithImage {
                                    image,
                                    transform: wl_transform_to_frame_transform(
                                        // does this even matter
                                        attrs.buffer_transform,
                                    ),
                                    scale: attrs.buffer_scale,
                                    dmabuf: false,
                                };
                                sbwi.apply_to_surface(states, None);
                            }
                            buffer.release();
                        }
                        Some(other) => log::warn!("Unsupported wl_buffer format: {other:?}"),
                        None => { /* don't draw anything */ }
                    }
                }
                Some(BufferAssignment::Removed) | None => {}
            }

            let callbacks = std::mem::take(&mut attrs.frame_callbacks);
            if !callbacks.is_empty() {
                self.pending_frame_callbacks
                    .entry(surface.id())
                    .or_default()
                    .extend(callbacks);
            }
        });

        self.redraw_requests.insert(surface.id());
    }
}

impl SeatHandler for Application {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_image = image;
    }
}

impl BufferHandler for Application {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ClientDndGrabHandler for Application {}

impl ServerDndGrabHandler for Application {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

impl DataDeviceHandler for Application {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device
    }
}

impl SelectionHandler for Application {
    type SelectionUserData = Arc<[u8]>;

    fn send_selection(
        &mut self,
        _ty: selection::SelectionTarget,
        _mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        let buf = user_data.clone();
        std::thread::spawn(move || {
            // Clear O_NONBLOCK, otherwise File::write_all() will stop halfway.
            if let Err(err) = fcntl_setfl(&fd, OFlags::empty()) {
                log::warn!("error clearing flags on selection target fd: {err:?}");
            }
            if let Err(err) = File::from(fd).write_all(&buf) {
                log::warn!("error writing selection: {err:?}");
            }
        });
    }

    fn new_selection(
        &mut self,
        _ty: selection::SelectionTarget,
        _source: Option<selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
    }
}

#[derive(Default)]
pub struct ClientState {
    compositor_state: compositor::CompositorClientState,
    pub disconnected: Arc<Mutex<bool>>,
}

impl ClientData for ClientState {
    fn initialized(&self, client_id: ClientId) {
        log::debug!("Client ID {client_id:?} connected");
    }

    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        *self.disconnected.lock().unwrap() = true;
        log::debug!("Client ID {client_id:?} disconnected. Reason: {reason:?}");
    }
}

impl AsMut<compositor::CompositorState> for Application {
    fn as_mut(&mut self) -> &mut compositor::CompositorState {
        &mut self.compositor
    }
}

impl XdgShellHandler for Application {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();
        self.send_initial_surface_state(wl_surface);
        if let Some(client) = wl_surface.client() {
            self.wayvr_tasks
                .send(WayVRTask::NewToplevel(client.id(), surface.clone()));
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.output.leave(surface.wl_surface());

        if let Some(client) = surface.wl_surface().client() {
            self.wayvr_tasks
                .send(WayVRTask::DropToplevel(client.id(), surface.clone()));
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.constrain_popup(&surface);

        let _ = self
            .popup_manager
            .track_popup(PopupKind::Xdg(surface))
            .context("Could not track xdg_popup")
            .inspect_err(|e| log::warn!("{e:?}"));
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        self.popup_manager.cleanup();
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        log::info!(
            "xdg_popup.grab: surface={:?} serial={:?}",
            surface.wl_surface().id(),
            serial
        );

        let popup = PopupKind::Xdg(surface.clone());

        let Ok(root_surface) = find_popup_root_surface(&popup) else {
            log::warn!("xdg_popup.grab: could not find popup root surface");
            return;
        };

        let Some(seat) = Seat::<Application>::from_resource(&seat) else {
            log::warn!("xdg_popup.grab: unknown seat");
            return;
        };

        let root_focus = root_surface;

        let Ok(mut grab) = self
            .popup_manager
            .grab_popup::<Application>(root_focus, popup, &seat, serial)
        else {
            log::debug!("xdg_popup.grab denied");
            return;
        };

        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial)
                    || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }

            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }

        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial)
                    || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }

            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });

        self.constrain_popup(&surface);
        surface.send_repositioned(token);
    }

    // If the app requests fullscreen, make it fill the virtual output
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<wl_output::WlOutput>,
    ) {
        let size = self.output_logical_size();

        surface.with_pending_state(|state| {
            state.bounds = Some(size);
            state.size = Some(size);
            state.states.set(xdg_toplevel::State::Fullscreen);
        });

        surface.send_configure();
    }

    // If the app requests maximize, make it fill the virtual output
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let size = self.output_logical_size();

        surface.with_pending_state(|state| {
            state.bounds = Some(size);
            state.size = Some(size);
            state.states.set(xdg_toplevel::State::Maximized);
        });

        surface.send_configure();
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        let bounds = self.output_logical_size();

        surface.with_pending_state(|state| {
            state.bounds = Some(bounds);
            state.size = None;
            state.states.unset(xdg_toplevel::State::Fullscreen);
        });

        surface.send_configure();
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        let bounds = self.output_logical_size();

        surface.with_pending_state(|state| {
            state.bounds = Some(bounds);
            state.size = None;
            state.states.unset(xdg_toplevel::State::Maximized);
        });

        surface.send_configure();
    }

    // If the app requests minimize, hide its window
    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(client) = surface.wl_surface().client() {
            self.wayvr_tasks
                .send(WayVRTask::MinimizeRequest(client.id(), surface.clone()));
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(client) = surface.wl_surface().client() {
            self.wayvr_tasks
                .send(WayVRTask::TitleChange(client.id(), surface.clone()));
        }
    }
}

impl ShmHandler for Application {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}

impl OutputHandler for Application {}

impl DmabufHandler for Application {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state.0
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if self.image_importer.get_or_import_dmabuf(dmabuf).is_ok() {
            let _ = notifier.successful::<Self>();
        } else {
            notifier.failed();
        }
    }
}

impl PrimarySelectionHandler for Application {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl selection_wlr::DataControlHandler for Application {
    fn data_control_state(&self) -> &selection_wlr::DataControlState {
        &self.wlr_data_control_state
    }
}

impl selection_ext::DataControlHandler for Application {
    fn data_control_state(&self) -> &selection_ext::DataControlState {
        &self.ext_data_control_state
    }
}
impl XdgDecorationHandler for Application {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn request_mode(
        &mut self,
        toplevel: ToplevelSurface,
        _mode: zxdg_toplevel_decoration_v1::Mode,
    ) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        });
        toplevel.send_configure();
    }
}

impl KdeDecorationHandler for Application {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.kde_decoration_state
    }

    fn request_mode(
        &mut self,
        _surface: &WlSurface,
        decoration: &org_kde_kwin_server_decoration::OrgKdeKwinServerDecoration,
        _mode: wayland_server::WEnum<org_kde_kwin_server_decoration::Mode>,
    ) {
        decoration.mode(org_kde_kwin_server_decoration::Mode::Server);
    }
}

delegate_dmabuf!(Application);
delegate_xdg_shell!(Application);
delegate_compositor!(Application);
delegate_viewporter!(Application);
delegate_shm!(Application);
delegate_seat!(Application);
delegate_data_device!(Application);
delegate_output!(Application);
delegate_primary_selection!(Application);
delegate_data_control!(Application);
delegate_ext_data_control!(Application);
delegate_xdg_decoration!(Application);
delegate_kde_decoration!(Application);
delegate_single_pixel_buffer!(Application);
delegate_relative_pointer!(Application);

const fn wl_transform_to_frame_transform(
    transform: wl_output::Transform,
) -> wlx_capture::frame::Transform {
    match transform {
        wl_output::Transform::Normal => wlx_capture::frame::Transform::Normal,
        wl_output::Transform::_90 => wlx_capture::frame::Transform::Rotated90,
        wl_output::Transform::_180 => wlx_capture::frame::Transform::Rotated180,
        wl_output::Transform::_270 => wlx_capture::frame::Transform::Rotated270,
        wl_output::Transform::Flipped => wlx_capture::frame::Transform::Flipped,
        wl_output::Transform::Flipped90 => wlx_capture::frame::Transform::Flipped90,
        wl_output::Transform::Flipped180 => wlx_capture::frame::Transform::Flipped180,
        wl_output::Transform::Flipped270 => wlx_capture::frame::Transform::Flipped270,
        _ => wlx_capture::frame::Transform::Undefined,
    }
}
