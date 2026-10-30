# wlx-capture

This aims to be an all-in-one Linux desktop capture solution.

I'm primarily using this for various XR projects.

Supported capture methods:
- Pipewire (MemFd+MemPtr+DmaBuf)
- Wlr-Dmabuf (Sway, Hyprland, River etc)
- XSHM

# Early Development

This project is in a highly experimental state. If you want to talk about this project, find me on:

- Discord: https://discord.gg/gHwJ2vwSWV
- Matrix Space: `#linux-vr-adventures:matrix.org`

# Usage

### Pipewire Setup
```rust
let Ok(node_id) = pipewire_select_screen(None, true, true, true).await else {
    return;
};
let capture = PipewireCapture::new(
    "wlx-capture", // name of stream
    node_id,
    60, // fps to give to pipewire, no effect as of writing
);
```

### XSHM Setup
```rust
let monitors = XshmCapture::get_monitors();
let mut capture = XshmCapture::new(monitors[0]).unwrap();
```


### Receiving Frames
```rust
let frame_rx = capture.init();
capture.request_new_frame();
loop {
    if let Ok(frame) = frame_rx.try_recv() {
        match frame {
            WlxFrame::DmaBuf(dmabuf_frame) => {
                // vulkano: https://github.com/galister/wlx-overlay-s/blob/6ee0cb4fd1bd099a0f22171656f87f85e3f86abd/src/graphics.rs#L740
                // egl: https://github.com/galister/wlx-overlay-x/blob/04f5e90cf8248705010beaf35aed3cf22f0e62c1/src/desktop/frame.rs#L255
            }
            WlxFrame::MemFd(memfd_frame) => {
                // egl: https://github.com/galister/wlx-overlay-x/blob/04f5e90cf8248705010beaf35aed3cf22f0e62c1/src/desktop/frame.rs#L207
            }
            WlxFrame::MemPtr(memptr_frame) => {
                // egl: https://github.com/galister/wlx-overlay-x/blob/04f5e90cf8248705010beaf35aed3cf22f0e62c1/src/desktop/frame.rs#L185
            }
        }
        capture.request_new_frame();
    }
}

```

Notes: 
- `PipewireCapture` will produce frames on its own and doesn't require `request_new_frame`.
- You may call `request_new_frame` at any time after `init` without worrying if a frame capture is already in progress.
- Calling `request_new_frame` when a frame is not ready yet will return and not trigger another frame capture.
