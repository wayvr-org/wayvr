use anyhow::Context;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::{BufferType, buffer_type};
use smithay::desktop::{PopupKind, PopupManager};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::rustix::fs::{OFlags, fcntl_setfl};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_output, wl_seat};
use smithay::reexports::wayland_server::{self, DisplayHandle};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::dmabuf::{
    DmabufFeedback, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier, get_dmabuf,
};
use smithay::wayland::fractional_scale::with_fractional_scale;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::{
    ext_data_control as selection_ext,
    primary_selection::{PrimarySelectionHandler, PrimarySelectionState, set_primary_focus},
    wlr_data_control as selection_wlr,
};
use smithay::wayland::shell::kde::decoration::{KdeDecorationHandler, KdeDecorationState};
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shm::{ShmHandler, ShmState, with_buffer_contents};
use smithay::wayland::single_pixel_buffer::get_single_pixel_buffer;
use smithay::{
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_ext_data_control, delegate_kde_decoration, delegate_output,
    delegate_primary_selection, delegate_seat, delegate_shm, delegate_single_pixel_buffer,
    delegate_xdg_decoration, delegate_xdg_shell,
};
use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};

use smithay::utils::Serial;
use smithay::wayland::compositor::{self, BufferAssignment, SurfaceAttributes, send_surface_state};

use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
    set_data_device_focus,
};
use smithay::wayland::selection::{self, SelectionHandler};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use wayland_server::Client;
use wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use wayland_server::protocol::wl_surface::WlSurface;

use crate::backend::wayvr::image_importer::ImageImporter;
use crate::backend::wayvr::{SurfaceBufWithImage, time};
use crate::ipc::event_queue::SyncEventQueue;

use super::WayVRTask;

pub struct Application {
    pub image_importer: ImageImporter,
    pub dmabuf_state: (DmabufState, DmabufGlobal, Option<DmabufFeedback>),
    pub compositor: compositor::CompositorState,
    pub xdg_shell: XdgShellState,
    pub seat_state: SeatState<Application>,
    pub shm: ShmState,
    pub data_device: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub ext_data_control_state: selection_ext::DataControlState,
    pub wlr_data_control_state: selection_wlr::DataControlState,
    pub kde_decoration_state: KdeDecorationState,
    pub wayvr_tasks: SyncEventQueue<WayVRTask>,
    pub redraw_requests: HashSet<wayland_server::backend::ObjectId>,
    pub popup_manager: PopupManager,
    pub display_handle: DisplayHandle,
}

impl Application {
    pub fn cleanup(&mut self) {
        self.image_importer.cleanup();
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
                            if let Ok(image) = self
                                .image_importer
                                .get_or_import_dmabuf(dmabuf.clone())
                                .inspect_err(|e| {
                                    log::warn!("wayland_server failed to import DMA-buf: {e:?}");
                                })
                            {
                                let sbwi = SurfaceBufWithImage {
                                    image,
                                    transform: wl_transform_to_frame_transform(
                                        attrs.buffer_transform,
                                    ),
                                    scale: attrs.buffer_scale,
                                    dmabuf: true,
                                };
                                sbwi.apply_to_surface(states);
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
                                    sbwi.apply_to_surface(states);
                                }
                            });
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
                                sbwi.apply_to_surface(states);
                            }
                        }
                        Some(other) => log::warn!("Unsupported wl_buffer format: {other:?}"),
                        None => { /* don't draw anything */ }
                    }
                    buffer.release();
                }
                Some(BufferAssignment::Removed) | None => {}
            }

            let t = time::get_millis() as u32;
            let callbacks = std::mem::take(&mut attrs.frame_callbacks);
            for cb in callbacks {
                cb.done(t);
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

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
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
        if let Some(client) = surface.wl_surface().client() {
            self.wayvr_tasks
                .send(WayVRTask::NewToplevel(client.id(), surface.clone()));
        }
        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
        });
        surface.send_configure();
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(client) = surface.wl_surface().client() {
            self.wayvr_tasks
                .send(WayVRTask::DropToplevel(client.id(), surface.clone()));
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let _ = self
            .popup_manager
            .track_popup(PopupKind::Xdg(surface))
            .context("Could not track xdg_popup")
            .inspect_err(|e| log::warn!("{e:?}"));
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        self.popup_manager.cleanup();
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // Handle popup grab here
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        // Handle popup reposition here
    }

    // If the app wants to be fullscreen, make it think that it's fullscreen.
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<wl_output::WlOutput>,
    ) {
        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();
    }
    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();
    }
    // If the app wants to be maximized, make it think that it's maximized.
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Maximized);
        });
        surface.send_configure();
    }
    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| {
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
    }

    fn request_mode(
        &mut self,
        _toplevel: ToplevelSurface,
        _mode: zxdg_toplevel_decoration_v1::Mode,
    ) {
        // no switching away from SSD
    }

    fn unset_mode(&mut self, _toplevel: ToplevelSurface) {
        // no switching away from SSD
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
