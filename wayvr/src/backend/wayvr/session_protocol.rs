#[allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
#[allow(non_upper_case_globals, non_snake_case, unused_imports)]
#[allow(clippy::all)]
pub mod server {
    use smithay::reexports::wayland_server;
    use wayland_protocols::xdg::shell::server::xdg_toplevel;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use super::wayland_server;
        use wayland_protocols::xdg::shell::server::__interfaces::{
            XDG_TOPLEVEL_INTERFACE, xdg_toplevel_interface,
        };
        use wayland_server::backend as wayland_backend;
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/xdg-session-management-v1.xml");
    }
    use self::__interfaces::{
        XDG_SESSION_MANAGER_V1_INTERFACE, XDG_SESSION_V1_INTERFACE,
        XDG_TOPLEVEL_SESSION_V1_INTERFACE,
    };
    wayland_scanner::generate_server_code!("protocols/xdg-session-management-v1.xml");
}
