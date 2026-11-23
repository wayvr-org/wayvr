use xkbcommon::xkb::{
    self,
    x11::{
        MIN_MAJOR_XKB_VERSION, MIN_MINOR_XKB_VERSION, SetupXkbExtensionFlags,
        get_core_keyboard_device_id, keymap_new_from_device, setup_xkb_extension,
    },
};

use super::XkbKeymap;

pub fn get_keymap_x11() -> anyhow::Result<XkbKeymap> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

    let (conn, _) = xcb::Connection::connect(None)?;
    setup_xkb_extension(
        &conn,
        MIN_MAJOR_XKB_VERSION,
        MIN_MINOR_XKB_VERSION,
        SetupXkbExtensionFlags::NoFlags,
        &mut 0,
        &mut 0,
        &mut 0,
        &mut 0,
    );

    let device_id = get_core_keyboard_device_id(&conn);
    if device_id == -1 {
        return Err(anyhow::anyhow!(
            "get_core_keyboard_device_id returned -1. Check your XKB installation."
        ));
    }
    let keymap = keymap_new_from_device(&context, &conn, device_id, xkb::KEYMAP_COMPILE_NO_FLAGS);

    Ok(XkbKeymap {
        inner: keymap,
        layout_code: "X11".into(),
    })
}
