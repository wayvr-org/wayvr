use std::{
    any::Any,
    collections::VecDeque,
    os::fd::{FromRawFd, IntoRawFd, OwnedFd, RawFd},
    sync::mpsc,
    thread::JoinHandle,
};

use drm_fourcc::{DrmFormat, DrmFourcc, DrmModifier};
use smithay_client_toolkit::reexports::protocols_wlr::export_dmabuf::v1::client::zwlr_export_dmabuf_frame_v1::{self, ZwlrExportDmabufFrameV1};
use wayland_client::{Connection, QueueHandle, Dispatch, Proxy};

use crate::{
    WlxCapture,
    frame::{DmabufFrame, FrameFormat, FramePlane, WlxFrame},
    wayland::WlxClient,
};

use log::{debug, warn};

struct CaptureData<U, R>
where
    U: Any,
    R: Any,
{
    sender: mpsc::SyncSender<WlxFrame>,
    receiver: mpsc::Receiver<WlxFrame>,
    user_data: U,
    receive_callback: fn(&U, WlxFrame) -> Option<R>,
}

pub struct WlrDmabufCapture<U, R>
where
    U: Any + Send,
    R: Any + Send,
{
    output_id: u32,
    wl: Option<Box<WlxClient>>,
    handle: Option<JoinHandle<Box<WlxClient>>>,
    data: Option<CaptureData<U, R>>,
    fds: VecDeque<RawFd>,
}

impl<U, R> WlrDmabufCapture<U, R>
where
    U: Any + Send,
    R: Any + Send,
{
    pub fn new(wl: WlxClient, output_id: u32) -> Self {
        Self {
            output_id,
            wl: Some(Box::new(wl)),
            handle: None,
            data: None,
            fds: VecDeque::new(),
        }
    }
}

impl<U, R> WlxCapture<U, R> for WlrDmabufCapture<U, R>
where
    U: Any + Send,
    R: Any + Send,
{
    fn init(
        &mut self,
        _: &[DrmFormat],
        user_data: U,
        receive_callback: fn(&U, WlxFrame) -> Option<R>,
    ) {
        debug_assert!(self.wl.is_some());

        let (sender, receiver) = std::sync::mpsc::sync_channel::<WlxFrame>(2);
        self.data = Some(CaptureData {
            sender,
            receiver,
            user_data,
            receive_callback,
        });
    }
    fn is_ready(&self) -> bool {
        self.data.is_some()
    }
    fn supports_dmbuf(&self) -> bool {
        true
    }
    fn receive(&mut self) -> Option<R> {
        if let Some(data) = self.data.as_ref()
            && let Some(WlxFrame::Dmabuf(last)) = data.receiver.try_iter().last()
        {
            // this is the only protocol that requires us to manually close the FD
            while self.fds.len() > 6 * last.num_planes {
                // safe unwrap
                let _ = unsafe { OwnedFd::from_raw_fd(self.fds.pop_back().unwrap()) };
            }
            for p in 0..last.num_planes {
                if let Some(fd) = last.planes[p].fd {
                    self.fds.push_front(fd);
                }
            }
            return (data.receive_callback)(&data.user_data, WlxFrame::Dmabuf(last));
        }
        None
    }
    fn pause(&mut self) {}
    fn resume(&mut self) {
        self.receive(); // clear old frames
    }
    fn request_new_frame(&mut self) {
        if let Some(handle) = self.handle.take() {
            if handle.is_finished() {
                self.wl = Some(handle.join().unwrap()); // safe to unwrap because we checked is_finished
            } else {
                self.handle = Some(handle);
                return;
            }
        }

        let Some(wl) = self.wl.take() else {
            return;
        };

        self.handle = Some(std::thread::spawn({
            let sender = self
                .data
                .as_ref()
                .expect("must call init once before request_new_frame")
                .sender
                .clone();
            let output_id = self.output_id;
            move || request_dmabuf_frame(wl, output_id, sender)
        }));
    }
}

/// Request a new DMA-Buf frame using the wlr-export-dmabuf protocol.
fn request_dmabuf_frame(
    client: Box<WlxClient>,
    output_id: u32,
    sender: mpsc::SyncSender<WlxFrame>,
) -> Box<WlxClient> {
    let Some(dmabuf_manager) = client.maybe_wlr_dmabuf_mgr.as_ref() else {
        return client;
    };

    let Some(output) = client.outputs.get(output_id) else {
        return client;
    };

    let transform = output.transform;

    let (tx, rx) = mpsc::sync_channel::<zwlr_export_dmabuf_frame_v1::Event>(16);
    let name = output.name.clone();

    let _ = dmabuf_manager.capture_output(1, &output.wl_output, &client.queue_handle, tx.clone());

    let mut client = client;
    client.dispatch();

    let mut frame = None;

    rx.try_iter().for_each(|event| match event {
        zwlr_export_dmabuf_frame_v1::Event::Frame {
            width,
            height,
            format,
            mod_high,
            mod_low,
            num_objects,
            ..
        } => {
            let mut new_frame = DmabufFrame {
                format: FrameFormat {
                    width,
                    height,
                    drm_format: DrmFormat {
                        code: match DrmFourcc::try_from(format) {
                            Ok(code) => code,
                            Err(e) => {
                                log::error!("Unrecognized fourcc: {e:?}");
                                return;
                            }
                        },
                        modifier: DrmModifier::Invalid,
                    },
                    transform,
                },
                mouse: None,
                num_planes: num_objects as _,
                planes: Default::default(),
            };
            new_frame.format.set_mod(mod_high, mod_low);
            frame = Some(new_frame);
        }
        zwlr_export_dmabuf_frame_v1::Event::Object {
            index,
            fd,
            offset,
            stride,
            ..
        } => {
            let Some(ref mut frame) = frame else {
                return;
            };
            frame.planes[index as usize] = FramePlane {
                fd: Some(fd.into_raw_fd()),
                offset,
                stride: stride as _,
            };
        }
        zwlr_export_dmabuf_frame_v1::Event::Ready { .. } => {
            let Some(frame) = frame.take() else {
                return;
            };
            debug!("DMA-Buf frame captured");
            let frame = WlxFrame::Dmabuf(frame);
            match sender.try_send(frame) {
                Ok(_) => (),
                Err(mpsc::TrySendError::Full(_)) => (),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    log::warn!("{}: disconnected", name);
                }
            }
        }
        zwlr_export_dmabuf_frame_v1::Event::Cancel { .. } => {
            warn!("DMA-Buf frame capture cancelled");
        }
        _ => {}
    });

    client
}

impl Dispatch<ZwlrExportDmabufFrameV1, mpsc::SyncSender<zwlr_export_dmabuf_frame_v1::Event>>
    for WlxClient
{
    fn event(
        _state: &mut Self,
        proxy: &ZwlrExportDmabufFrameV1,
        event: <ZwlrExportDmabufFrameV1 as Proxy>::Event,
        data: &mpsc::SyncSender<zwlr_export_dmabuf_frame_v1::Event>,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_export_dmabuf_frame_v1::Event::Ready { .. }
            | zwlr_export_dmabuf_frame_v1::Event::Cancel { .. } => {
                proxy.destroy();
            }
            _ => {}
        }

        let _ = data.send(event).or_else(|err| {
            warn!("Failed to send DMA-Buf frame event: {}", err);
            Ok::<(), mpsc::SendError<zwlr_export_dmabuf_frame_v1::Event>>(())
        });
    }
}
