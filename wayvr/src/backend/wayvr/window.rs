use std::{rc::Rc, sync::Arc};

use glam::DVec2;
use slotmap::{DenseSlotMap, new_key_type};
use smithay::utils::{Logical, Serial, Size};
use smithay::wayland::shell::xdg::ToplevelSurface;
use wayvr_ipc::packet_server;

use crate::backend::wayvr::process;

#[derive(Debug, Clone, Copy)]
pub struct PendingConfigure {
    pub size: Size<i32, Logical>,
    pub serial: Serial,
}

#[derive(Debug)]
pub struct Window {
    pub min_size: Size<i32, Logical>,
    pub max_size: Size<i32, Logical>,
    pub bounds: Size<i32, Logical>,
    pub size_x: u32,
    pub size_y: u32,
    pub visible: bool,
    pub toplevel: Rc<ToplevelSurface>,
    pub process: process::ProcessHandle,
    pub app_id: Option<Arc<str>>,
    pub parent: Option<WindowHandle>,
    pub name: Option<Arc<str>>,
    pub restore_on_spawn: bool,

    pub pending_configure: Option<PendingConfigure>,
}

impl Window {
    fn new(
        toplevel: Rc<ToplevelSurface>,
        process: process::ProcessHandle,
        bounds: Size<i32, Logical>,
        min_size: Size<i32, Logical>,
        max_size: Size<i32, Logical>,
        app_id: Option<Arc<str>>,
        parent: Option<WindowHandle>,
        name: Option<Arc<str>>,
        restore_on_spawn: bool,
    ) -> Self {
        Self {
            bounds,
            min_size,
            max_size: max_size.clamp(Size::new(160, 160), bounds),
            size_x: 0,
            size_y: 0,
            visible: true,
            toplevel,
            process,
            app_id,
            parent,
            name,
            restore_on_spawn,
            pending_configure: None,
        }
    }

    pub fn resizable(&self) -> bool {
        self.min_size != self.max_size
    }

    pub fn clamp_configure_size(
        &self,
        size: Size<i32, Logical>,
        bounds: Size<i32, Logical>,
    ) -> Size<i32, Logical> {
        let min_size = Size::new(
            self.min_size.w.max(1).min(bounds.w),
            self.min_size.h.max(1).min(bounds.h),
        );

        let max_size = Size::new(
            self.max_size.w.max(min_size.w).min(bounds.w),
            self.max_size.h.max(min_size.h).min(bounds.h),
        );

        Size::new(size.w.max(1), size.h.max(1)).clamp(min_size, max_size)
    }

    fn send_size_configure(&mut self, size: Size<i32, Logical>, bounds: Size<i32, Logical>) {
        let clamped_size = self.clamp_configure_size(size, bounds);

        if self
            .pending_configure
            .is_some_and(|pending| pending.size == clamped_size)
        {
            return;
        }

        self.toplevel.with_pending_state(|state| {
            state.bounds = Some(bounds);
            state.size = Some(clamped_size);
        });

        let serial = self.toplevel.send_configure();

        self.bounds = bounds;
        self.pending_configure = Some(PendingConfigure {
            size: clamped_size,
            serial,
        });
    }

    pub fn checked_configure_size(&mut self, size: Size<i32, Logical>) {
        self.send_size_configure(size, self.bounds);
    }

    pub fn request_size(&mut self, size: Size<i32, Logical>, bounds: Size<i32, Logical>) {
        self.send_size_configure(size, bounds);
    }

    pub fn remember_committed_size(
        &mut self,
        size: Size<i32, Logical>,
        configure_serial: Option<Serial>,
    ) -> bool {
        let size_x = size.w.max(1) as u32;
        let size_y = size.h.max(1) as u32;

        let changed = self.size_x != size_x || self.size_y != size_y;

        self.size_x = size_x;
        self.size_y = size_y;

        if self
            .pending_configure
            .is_some_and(|pending| Some(pending.serial) == configure_serial)
        {
            self.pending_configure = None;
        }

        changed
    }
}

#[derive(Debug)]
pub struct MouseState {
    pub hover_window: WindowHandle,
    pub pos: DVec2,
}

#[derive(Debug)]
pub struct WindowManager {
    pub windows: DenseSlotMap<WindowHandle, Window>,
    pub mouse: Option<MouseState>,
    pub keyboard_focus: Option<WindowHandle>,
}

pub struct CreateWindowParams {
    pub toplevel: Rc<ToplevelSurface>,
    pub process: process::ProcessHandle,
    pub bounds: Size<i32, Logical>,
    pub min_size: Size<i32, Logical>,
    pub max_size: Size<i32, Logical>,
    pub size_x: u32,
    pub size_y: u32,
    pub app_id: Option<Arc<str>>,
    pub parent: Option<WindowHandle>,
    pub name: Option<Arc<str>>,
    pub restore_on_spawn: bool,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            windows: Default::default(),
            mouse: None,
            keyboard_focus: None,
        }
    }

    pub fn find_window_handle(&self, toplevel: &ToplevelSurface) -> Option<WindowHandle> {
        for (handle, window) in &self.windows {
            if *window.toplevel == *toplevel {
                return Some(handle);
            }
        }
        None
    }

    pub fn create_window(&mut self, par: CreateWindowParams) -> WindowHandle {
        let mut window = Window::new(
            par.toplevel,
            par.process,
            par.bounds,
            par.min_size,
            par.max_size,
            par.app_id,
            par.parent,
            par.name,
            par.restore_on_spawn,
        );
        window.remember_committed_size(Size::new(par.size_x as i32, par.size_y as i32), None);
        self.windows.insert(window)
    }

    pub fn remove_window(&mut self, window_handle: WindowHandle) {
        self.windows.remove(window_handle);
    }
}

new_key_type! {
    pub struct WindowHandle;
}

impl WindowHandle {
    pub fn from_packet(handle: packet_server::WvrWindowHandle) -> Self {
        Self::from(slotmap::KeyData::from_ffi(handle.user))
    }

    pub fn as_packet(self) -> packet_server::WvrWindowHandle {
        packet_server::WvrWindowHandle {
            user: self.0.as_ffi(),
        }
    }

    pub fn transient_name(self) -> Arc<str> {
        format!("wayvr/volatile/{}", self.as_packet().user).into()
    }
}
