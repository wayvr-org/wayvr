use std::rc::Rc;

use glam::DVec2;
use smithay::utils::{Logical, Size};
use smithay::wayland::shell::xdg::ToplevelSurface;
use wayvr_ipc::packet_server;

use crate::{backend::wayvr::process, gen_id};

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

    pub pending_configure_size: Option<Size<i32, Logical>>,
}

impl Window {
    fn new(
        toplevel: Rc<ToplevelSurface>,
        process: process::ProcessHandle,
        bounds: Size<i32, Logical>,
        min_size: Size<i32, Logical>,
        max_size: Size<i32, Logical>,
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
            pending_configure_size: None,
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

        if self.pending_configure_size == Some(clamped_size) {
            return;
        }

        self.toplevel.with_pending_state(|state| {
            state.bounds = Some(bounds);
            state.size = Some(clamped_size);
        });

        self.toplevel.send_configure();

        self.bounds = bounds;
        self.pending_configure_size = Some(clamped_size);
    }

    pub fn checked_configure_size(&mut self, size: Size<i32, Logical>) {
        self.send_size_configure(size, self.bounds);
    }

    pub fn request_size(&mut self, size: Size<i32, Logical>, bounds: Size<i32, Logical>) {
        self.send_size_configure(size, bounds);
    }

    pub fn remember_committed_size(&mut self, size: Size<i32, Logical>) -> bool {
        let size_x = size.w.max(1) as u32;
        let size_y = size.h.max(1) as u32;

        let changed = self.size_x != size_x || self.size_y != size_y;

        self.size_x = size_x;
        self.size_y = size_y;

        self.pending_configure_size = None;

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
    pub windows: WindowVec,
    pub mouse: Option<MouseState>,
    pub keyboard_focus: Option<WindowHandle>,
}

impl WindowManager {
    pub const fn new() -> Self {
        Self {
            windows: WindowVec::new(),
            mouse: None,
            keyboard_focus: None,
        }
    }

    pub fn find_window_handle(&self, toplevel: &ToplevelSurface) -> Option<WindowHandle> {
        for (idx, cell) in self.windows.vec.iter().enumerate() {
            if let Some(cell) = cell {
                let window = &cell.obj;
                if *window.toplevel == *toplevel {
                    return Some(WindowVec::get_handle(cell, idx));
                }
            }
        }
        None
    }

    pub fn create_window(
        &mut self,
        toplevel: Rc<ToplevelSurface>,
        process: process::ProcessHandle,
        bounds: Size<i32, Logical>,
        min_size: Size<i32, Logical>,
        max_size: Size<i32, Logical>,
        size_x: u32,
        size_y: u32,
    ) -> WindowHandle {
        let mut window = Window::new(toplevel, process, bounds, min_size, max_size);
        window.remember_committed_size(Size::new(size_x as i32, size_y as i32));
        self.windows.add(window)
    }

    pub fn remove_window(&mut self, window_handle: WindowHandle) {
        self.windows.remove(&window_handle);
    }
}

gen_id!(WindowVec, Window, WindowCell, WindowHandle);

impl WindowHandle {
    pub const fn from_packet(handle: packet_server::WvrWindowHandle) -> Self {
        Self {
            generation: handle.generation,
            idx: handle.idx,
        }
    }

    pub const fn as_packet(&self) -> packet_server::WvrWindowHandle {
        packet_server::WvrWindowHandle {
            idx: self.idx,
            generation: self.generation,
        }
    }
}
