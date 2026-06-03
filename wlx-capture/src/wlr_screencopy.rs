use drm_fourcc::{DrmFormat, DrmFourcc, DrmModifier};
use libc::{O_CREAT, O_RDWR, S_IRUSR, S_IWUSR};
use std::{
    any::Any,
    ffi::CString,
    os::fd::{BorrowedFd, RawFd},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::JoinHandle,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::{wl_buffer::WlBuffer, wl_shm::Format, wl_shm_pool::WlShmPool},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1;

use smithay_client_toolkit::reexports::protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{ZwlrScreencopyFrameV1, self};

use crate::{
    WlxCapture,
    frame::{DmaExporter, FrameFormat, FramePlane, MemFdFrame, WlxFrame},
    wayland::WlxClient,
};

enum BufData {
    Shm {
        wl_buffer: WlBuffer,
        wl_pool: WlShmPool,
        fd: RawFd,
    },
    Dma {
        wl_buffer: WlBuffer,
    },
}

impl Drop for BufData {
    fn drop(&mut self) {
        match self {
            Self::Shm {
                wl_buffer,
                wl_pool,
                fd,
                ..
            } => {
                wl_buffer.destroy();
                wl_pool.destroy();
                unsafe {
                    libc::close(*fd);
                }
            }
            Self::Dma { wl_buffer } => {
                wl_buffer.destroy();
            }
        }
    }
}

enum ScreenCopyEvent {
    Buffer {
        shm_format: Format,
        width: u32,
        height: u32,
        stride: u32,
    },
    DmaBuf {
        format: DrmFourcc,
        width: u32,
        height: u32,
    },
    BuffersDone,
    Ready,
    Failed,
}

struct CaptureData<U, R> {
    sender: mpsc::SyncSender<R>,
    receiver: mpsc::Receiver<R>,
    user_data: Option<Box<U>>,
    receive_callback: fn(&U, WlxFrame) -> Option<R>,
}

pub struct WlrScreencopyCapture<U, R> {
    output_id: u32,
    wl: Option<Box<WlxClient>>,
    handle: Option<JoinHandle<(Box<WlxClient>, Box<U>)>>,
    data: Option<CaptureData<U, R>>,
}

impl<U, R> WlrScreencopyCapture<U, R>
where
    U: Any + Send + DmaExporter,
    R: Any + Send,
{
    pub fn new(wl: WlxClient, output_id: u32) -> Self {
        Self {
            output_id,
            wl: Some(Box::new(wl)),
            handle: None,
            data: None,
        }
    }
}

impl<U, R> WlxCapture<U, R> for WlrScreencopyCapture<U, R>
where
    U: Any + Send + DmaExporter,
    R: Any + Send,
{
    fn init(
        &mut self,
        _: &[DrmFormat],
        user_data: U,
        receive_callback: fn(&U, WlxFrame) -> Option<R>,
    ) {
        debug_assert!(self.wl.is_some());

        let (sender, receiver) = mpsc::sync_channel(2);
        self.data = Some(CaptureData {
            sender,
            receiver,
            user_data: Some(Box::new(user_data)),
            receive_callback,
        });
    }
    fn is_ready(&self) -> bool {
        self.data.is_some()
    }
    fn supports_dmbuf(&self) -> bool {
        true // screencopy v3+
    }
    fn receive(&mut self) -> Option<R> {
        if let Some(data) = self.data.as_ref() {
            data.receiver.try_iter().last()
        } else {
            None
        }
    }
    fn pause(&mut self) {}
    fn resume(&mut self) {
        if self.data.is_none() {
            return;
        }
        self.receive(); // clear old frames
        self.request_new_frame();
    }
    fn request_new_frame(&mut self) {
        let mut wait_for_damage = false;
        if let Some(handle) = self.handle.take() {
            if handle.is_finished() {
                wait_for_damage = true;
                let (wl, u) = handle.join().unwrap(); // safe to unwrap because is_finished
                self.wl = Some(wl);
                self.data.as_mut().unwrap().user_data = Some(u);
            } else {
                self.handle = Some(handle);
                return;
            }
        }

        let Some(wl) = self.wl.take() else {
            return;
        };

        let data = self
            .data
            .as_mut()
            .expect("must call init once before request_new_frame");

        self.handle = Some(std::thread::spawn({
            let sender = data.sender.clone();
            let user_data = data.user_data.take().unwrap();
            let receive_callback = data.receive_callback;

            let output_id = self.output_id;
            move || {
                request_screencopy_frame(
                    wl,
                    output_id,
                    sender,
                    user_data,
                    receive_callback,
                    wait_for_damage,
                )
            }
        }));
    }
}

/// Request a new DMA-Buf frame using the wlr-screencopy protocol.
fn request_screencopy_frame<U, R>(
    client: Box<WlxClient>,
    output_id: u32,
    sender: SyncSender<R>,
    mut user_data: Box<U>,
    receive_callback: fn(&U, WlxFrame) -> Option<R>,
    wait_for_damage: bool,
) -> (Box<WlxClient>, Box<U>)
where
    U: Any + Send + DmaExporter,
    R: Any + Send,
{
    let Some(screencopy_manager) = client.maybe_wlr_screencopy_mgr.as_ref() else {
        return (client, user_data);
    };

    let Some(output) = client.outputs.get(output_id) else {
        return (client, user_data);
    };

    let transform = output.transform;

    let (tx, rx) = mpsc::sync_channel::<ScreenCopyEvent>(16);

    let proxy =
        screencopy_manager.capture_output(1, &output.wl_output, &client.queue_handle, tx.clone());

    let name = output.name.clone();

    let mut client = client;
    client.dispatch();

    let mut frame_buffer = None;

    let mut maybe_buffer = None;
    let mut maybe_dmabuf = None;

    'receiver: loop {
        for event in rx.try_iter() {
            match event {
                ScreenCopyEvent::Buffer { .. } => {
                    log::trace!("{name}: ScreenCopy Buffer event received");
                    maybe_buffer = Some(event);
                }
                ScreenCopyEvent::DmaBuf { .. } => {
                    log::trace!("{name}: ScreenCopy LinuxDmabuf event received");
                    maybe_dmabuf = Some(event);
                }
                ScreenCopyEvent::BuffersDone => {
                    log::trace!("{name}: ScreenCopy BuffersDone event received");
                    if let Some(zwp_linux_dmabuf) = client.maybe_zwp_linux_dmabuf.as_ref()
                        && let Some(ScreenCopyEvent::DmaBuf {
                            format,
                            width,
                            height,
                        }) = maybe_dmabuf
                        && let Some((plane, modifier)) = user_data.next_frame(width, height, format)
                    {
                        let mod_hi = (u64::from(modifier) >> 32) as _;
                        let mod_lo = (u64::from(modifier) & 0xFFFFFFFF) as _;
                        let fd = unsafe { BorrowedFd::borrow_raw(plane.fd.unwrap()) };

                        let params = zwp_linux_dmabuf.create_params(&client.queue_handle, ());
                        params.add(fd, 0, plane.offset, plane.stride as _, mod_hi, mod_lo);

                        let wl_buffer = params.create_immed(
                            width as _,
                            height as _,
                            format as _,
                            zwp_linux_buffer_params_v1::Flags::empty(),
                            &client.queue_handle,
                            (),
                        );

                        log::trace!("{name}: ScreenCopy with Dmabuf");
                        // copy_with_damage seems to not work here
                        proxy.copy(&wl_buffer);

                        frame_buffer =
                            Some((WlxFrame::Implicit(transform), BufData::Dma { wl_buffer }));
                    } else if let Some(ScreenCopyEvent::Buffer {
                        shm_format,
                        width,
                        height,
                        stride,
                    }) = maybe_buffer
                        && let Some(fourcc) = fourcc_from_wlshm(shm_format)
                    {
                        let fd_num = FD_COUNTER.fetch_add(1, Ordering::Relaxed);
                        let shm_name = CString::new(format!("wlx-{}", fd_num)).unwrap(); // safe
                        let size = stride * height;
                        let fd = unsafe {
                            let fd = libc::shm_open(
                                shm_name.as_ptr(),
                                O_CREAT | O_RDWR,
                                S_IRUSR | S_IWUSR,
                            );
                            libc::shm_unlink(shm_name.as_ptr());
                            libc::ftruncate(fd, size as _);
                            fd
                        };

                        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };

                        let wl_pool = client.wl_shm.create_pool(
                            borrowed_fd,
                            size as _,
                            &client.queue_handle,
                            (),
                        );

                        let wl_buffer = wl_pool.create_buffer(
                            0,
                            width as _,
                            height as _,
                            stride as _,
                            shm_format,
                            &client.queue_handle,
                            (),
                        );

                        log::trace!("{name}: ScreenCopy with SHM");
                        if wait_for_damage {
                            proxy.copy_with_damage(&wl_buffer);
                        } else {
                            proxy.copy(&wl_buffer);
                        }

                        let frame = MemFdFrame {
                            format: FrameFormat {
                                width,
                                height,
                                drm_format: DrmFormat {
                                    code: fourcc,
                                    modifier: DrmModifier::Invalid,
                                },
                                transform,
                            },
                            plane: FramePlane {
                                fd: Some(fd),
                                offset: 0,
                                stride: stride as _,
                            },
                            mouse: None,
                        };
                        frame_buffer = Some((
                            WlxFrame::MemFd(frame),
                            BufData::Shm {
                                wl_buffer,
                                wl_pool,
                                fd,
                            },
                        ));
                    } else {
                        log::error!("{name}: No usable ScreenCopy buffers received.");
                        proxy.destroy();
                        break 'receiver;
                    }

                    client.dispatch();
                }
                ScreenCopyEvent::Ready => {
                    log::trace!("{}: Frame ready?", name.as_ref());
                    if let Some((frame, buffer)) = frame_buffer {
                        if let Some(r) = receive_callback(&user_data, frame) {
                            let _ = sender.send(r);
                            log::trace!("{}: Frame ready!", name.as_ref());
                        }
                        drop(buffer);
                    }
                    break 'receiver;
                }
                ScreenCopyEvent::Failed => {
                    log::trace!("{}: Frame failed", name.as_ref());
                    break 'receiver;
                }
            };
        }
    }

    (client, user_data)
}

static FD_COUNTER: AtomicUsize = AtomicUsize::new(0);

impl Dispatch<ZwlrScreencopyFrameV1, SyncSender<ScreenCopyEvent>> for WlxClient {
    fn event(
        _state: &mut Self,
        proxy: &ZwlrScreencopyFrameV1,
        event: <ZwlrScreencopyFrameV1 as Proxy>::Event,
        data: &SyncSender<ScreenCopyEvent>,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Failed => {
                let _ = data.send(ScreenCopyEvent::Failed);
                proxy.destroy();
            }
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                let WEnum::Value(shm_format) = format else {
                    log::warn!("Unknown screencopy format");
                    let _ = data.send(ScreenCopyEvent::Failed);
                    proxy.destroy();
                    return;
                };

                let _ = data.send(ScreenCopyEvent::Buffer {
                    width,
                    height,
                    stride,
                    shm_format,
                });
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                let _ = data.send(ScreenCopyEvent::Ready);
                proxy.destroy();
            }
            zwlr_screencopy_frame_v1::Event::LinuxDmabuf {
                format,
                width,
                height,
            } => {
                let Ok(format) = DrmFourcc::try_from(format) else {
                    log::warn!("{format} is not a known FourCC");
                    return;
                };

                let _ = data.send(ScreenCopyEvent::DmaBuf {
                    width,
                    height,
                    format,
                });
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                let _ = data.send(ScreenCopyEvent::BuffersDone);
            }
            _ => {}
        }
    }
}

fn fourcc_from_wlshm(shm_format: Format) -> Option<DrmFourcc> {
    match shm_format {
        Format::Argb8888 => Some(DrmFourcc::Argb8888),
        Format::Xrgb8888 => Some(DrmFourcc::Xrgb8888),
        Format::Abgr8888 => Some(DrmFourcc::Abgr8888),
        Format::Xbgr8888 => Some(DrmFourcc::Xbgr8888),
        _ => None,
    }
}

// Plumbing below

impl Dispatch<WlShmPool, ()> for WlxClient {
    fn event(
        _state: &mut Self,
        _proxy: &WlShmPool,
        _event: <WlShmPool as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlBuffer, ()> for WlxClient {
    fn event(
        _state: &mut Self,
        _proxy: &WlBuffer,
        _event: <WlBuffer as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}
