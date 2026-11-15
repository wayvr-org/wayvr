use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;

use drm_fourcc::DrmFormat;
use drm_fourcc::DrmFourcc;
use drm_fourcc::DrmModifier;
use pipewire as pw;
use pipewire::context::ContextRc;
use pipewire::main_loop::MainLoopRc;
use pipewire::spa::buffer::meta::{
    MetaCursor, MetaHeader, MetaHeaderFlags, MetaVideoTransform, MetaVideoTransformValue,
};
use pipewire::stream::StreamRc;
use pw::spa;

use pw::Error;
use pw::properties::properties;
use pw::stream::{Stream, StreamFlags};
use spa::buffer::DataType;
use spa::param::ParamType;
use spa::param::video::VideoFormat;
use spa::param::video::{VideoFlags, VideoInfoRaw};
use spa::pod::ChoiceValue;
use spa::pod::Pod;
use spa::pod::serialize::GenError;
use spa::pod::{Object, Property, PropertyFlags, Value};
use spa::utils::Choice;
use spa::utils::ChoiceEnum;
use spa::utils::ChoiceFlags;
use timeline_syncobj::render_node::DrmRenderNode;
use wayland_client::backend::smallvec::SmallVec;

use super::raw_buffer::DequeuedBuffer;
use crate::WlxCapture;
use crate::frame::FrameFormat;
use crate::frame::MouseMeta;
use crate::frame::Transform;
use crate::frame::WlxFrame;
use crate::frame::{
    DmabufFrame, FrameAcquireSync, FrameLease, FramePlane, MemFdFrame, MemPtrFrame,
};
use crate::pipewire::raw_buffer::{HeldBuffers, MAX_PW_BUFFERS};
use crate::syncobj::import_timeline;

const SPA_DATA_SYNCOBJ: u32 = 5;
const SPA_META_SYNC_TIMELINE: u32 = 9;
const SPA_PARAM_BUFFERS_META_TYPE: u32 = 7;

#[derive(Default)]
struct StreamData {
    format: Option<FrameFormat>,
    stream: Option<Stream>,
}

#[derive(Debug)]
pub enum PwChangeRequest {
    Pause,
    Resume,
    Stop,
    ReleaseBuffer(usize),
}

struct CaptureData<R>
where
    R: Any + Send,
{
    tx_ctrl: pw::channel::Sender<PwChangeRequest>,
    rx_frame: mpsc::Receiver<R>,
}

pub struct PipewireCapture<R>
where
    R: Any + Send,
{
    name: Arc<str>,
    data: Option<CaptureData<R>>,
    node_id: u32,
    handle: Option<JoinHandle<Result<(), Error>>>,
    drm_render_node: Option<DrmRenderNode>,
}

impl<R> PipewireCapture<R>
where
    R: Any + Send,
{
    pub fn new(name: Arc<str>, node_id: u32, drm_render_node: Option<DrmRenderNode>) -> Self {
        PipewireCapture {
            name,
            data: None,
            node_id,
            handle: None,
            drm_render_node,
        }
    }
}

impl<R> Drop for PipewireCapture<R>
where
    R: Any + Send,
{
    fn drop(&mut self) {
        if let Some(data) = self.data.take() {
            drop(data.rx_frame);
            let _ = data.tx_ctrl.send(PwChangeRequest::Stop);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl<U, R> WlxCapture<U, R> for PipewireCapture<R>
where
    U: Any + Send,
    R: Any + Send,
{
    fn init(
        &mut self,
        dmabuf_formats: &[DrmFormat],
        user_data: U,
        receive_callback: fn(&U, WlxFrame) -> Option<R>,
    ) {
        let (tx_frame, rx_frame) = mpsc::sync_channel(4);
        let (tx_ctrl, rx_ctrl) = pw::channel::channel();

        self.data = Some(CaptureData {
            tx_ctrl: tx_ctrl.clone(),
            rx_frame,
        });

        let drm_render_node = self.drm_render_node.clone();

        self.handle = Some(std::thread::spawn({
            let name = self.name.clone();
            let node_id = self.node_id;
            let formats = dmabuf_formats.to_vec();

            move || {
                main_loop::<U, R>(
                    name,
                    node_id,
                    formats,
                    tx_frame,
                    tx_ctrl,
                    rx_ctrl,
                    user_data,
                    receive_callback,
                    drm_render_node,
                )
            }
        }));
    }
    fn is_ready(&self) -> bool {
        self.data.is_some()
    }
    fn supports_dmbuf(&self) -> bool {
        true
    }
    fn receive(&mut self) -> Option<R> {
        if let Some(data) = self.data.as_ref() {
            return data.rx_frame.try_iter().last();
        }
        None
    }
    fn pause(&mut self) {
        if let Some(data) = &self.data {
            match data.tx_ctrl.send(PwChangeRequest::Pause) {
                Ok(_) => (),
                Err(_) => {
                    log::warn!("{}: disconnected, stopping stream", self.name);
                }
            }
        }
    }
    fn resume(&mut self) {
        if let Some(data) = &self.data {
            match data.tx_ctrl.send(PwChangeRequest::Resume) {
                Ok(_) => {
                    log::debug!(
                        "{}: dropped {} old frames before resuming",
                        self.name,
                        data.rx_frame.try_iter().count()
                    );
                }
                Err(_) => {
                    log::warn!("{}: disconnected, stopping stream", self.name);
                }
            }
        }
    }
    fn request_new_frame(&mut self) {}
}

fn main_loop<U, R>(
    name: Arc<str>,
    node_id: u32,
    dmabuf_formats: Vec<DrmFormat>,
    tx_frame: mpsc::SyncSender<R>,
    tx_ctrl: pw::channel::Sender<PwChangeRequest>,
    rx_ctrl: pw::channel::Receiver<PwChangeRequest>,
    user_data: U,
    receive_callback: fn(&U, WlxFrame) -> Option<R>,
    drm_render_node: Option<DrmRenderNode>,
) -> Result<(), Error>
where
    U: Any,
    R: Any,
{
    log::debug!("{}: pipewire main_loop start", name);
    let main_loop = MainLoopRc::new(None)?;
    let context = ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;

    let held_buffers = Rc::new(RefCell::new(HeldBuffers::default()));

    let stream = StreamRc::new(
        core,
        &name,
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let _listener = stream
        .add_local_listener_with_user_data(FrameFormat {
            width: 0,
            height: 0,
            drm_format: DrmFormat {
                code: DrmFourcc::Argb8888,
                modifier: DrmModifier::Invalid,
            },
            transform: Transform::Undefined,
        })
        .state_changed({
            let name = name.clone();
            move |_, _, old, new| {
                log::info!("{}: stream state changed: {:?} -> {:?}", name, old, new);
            }
        })
        .param_changed({
            let name = name.clone();
            let support_explicit_sync = drm_render_node.is_some();
            move |stream, format, id, param| {
                let Some(param) = param else {
                    return;
                };
                if id != ParamType::Format.as_raw() {
                    return;
                }

                let mut info = VideoInfoRaw::default();
                info.parse(param)
                    .expect("Failed to parse param changed to VideoInfoRaw");

                format.width = info.size().width;
                format.height = info.size().height;
                format.drm_format.code = spa_to_fourcc(info.format());
                format.drm_format.modifier = DrmModifier::from(info.modifier());

                let dmabuf = info.flags().contains(VideoFlags::MODIFIER);
                let kind = if dmabuf { "DMA-buf" } else { "SHM" };

                log::info!("{}: got {} video format:", name, kind);
                log::info!("  format: {} ({:?})", info.format().as_raw(), info.format());
                log::info!("  size: {}x{}", info.size().width, info.size().height);
                log::info!("  modifier: {}", info.modifier());

                let mut param_bytes = Vec::new();
                if dmabuf && support_explicit_sync {
                    param_bytes.push(obj_to_bytes(get_sync_buffer_params()).unwrap());
                    // order defines preference, we prefer explicit sync
                    param_bytes.push(
                        obj_to_bytes(get_meta_object(
                            SPA_META_SYNC_TIMELINE,
                            std::mem::size_of::<MetaSyncTimeline>(),
                        ))
                        .unwrap(),
                    );
                }
                param_bytes.push(obj_to_bytes(get_buffer_params()).unwrap());
                param_bytes.push(
                    obj_to_bytes(get_meta_object(
                        spa::sys::SPA_META_Header,
                        std::mem::size_of::<spa::sys::spa_meta_header>(),
                    ))
                    .unwrap(),
                );
                param_bytes.push(
                    obj_to_bytes(get_meta_object(
                        spa::sys::SPA_META_VideoTransform,
                        std::mem::size_of::<spa::sys::spa_meta_videotransform>(),
                    ))
                    .unwrap(),
                );

                let mut pods: Vec<&Pod> = param_bytes
                    .iter()
                    .map(|bytes| Pod::from_bytes(bytes).unwrap())
                    .collect();
                if let Err(e) = stream.update_params(&mut pods) {
                    log::error!("{}: failed to update params: {}", name, e);
                }
            }
        })
        .remove_buffer({
            let held_buffers = held_buffers.clone();
            let name = name.clone();
            move |_, _, ptr| {
                if let Some(buffer) = NonNull::new(ptr)
                    && held_buffers.borrow_mut().on_removed(buffer)
                {
                    log::warn!("{name}: PipeWire removed a still-leased buffer");
                }
            }
        })
        .process({
            let name = name.clone();
            let sync_device = drm_render_node.clone();
            let held_buffers = held_buffers.clone();
            let tx_ctrl = tx_ctrl.clone();
            let u = user_data;
            move |stream, format| {
                let mut new_buffer = None;
                while let Some(buffer) = DequeuedBuffer::dequeue(stream) {
                    new_buffer = Some(buffer);
                }
                let Some(buffer) = new_buffer else { return };

                if let Some(header) = buffer.find_meta::<MetaHeader>()
                    && header.flags().contains(MetaHeaderFlags::CORRUPTED)
                {
                    log::warn!("{name}: PipeWire buffer is corrupt");
                    return;
                }

                if let Some(transform) = buffer.find_meta::<MetaVideoTransform>() {
                    format.transform = match transform.transform() {
                        MetaVideoTransformValue::NONE => Transform::Normal,
                        MetaVideoTransformValue::ROTATED90 => Transform::Rotated90,
                        MetaVideoTransformValue::ROTATED180 => Transform::Rotated180,
                        MetaVideoTransformValue::ROTATED270 => Transform::Rotated270,
                        MetaVideoTransformValue::FLIPPED => Transform::Flipped,
                        MetaVideoTransformValue::FLIPPED90 => Transform::Flipped90,
                        MetaVideoTransformValue::FLIPPED180 => Transform::Flipped180,
                        MetaVideoTransformValue::FLIPPED270 => Transform::Flipped270,
                        _ => Transform::Undefined,
                    };
                }

                let mouse_meta = buffer
                    .find_meta::<MetaCursor>()
                    .filter(|c| c.is_valid())
                    .map(|c| MouseMeta {
                        x: c.position().x as f32 / format.width as f32,
                        y: c.position().y as f32 / format.height as f32,
                    });

                let acquire_point = buffer
                    .find_meta::<MetaSyncTimeline>()
                    .map(|meta| meta.acquire_point);

                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    log::debug!("{name}: empty PipeWire frame");
                    return;
                }
                let is_explicit = datas.len() >= 3
                    && datas[datas.len() - 2].type_().as_raw() == SPA_DATA_SYNCOBJ
                    && datas[datas.len() - 1].type_().as_raw() == SPA_DATA_SYNCOBJ;
                if is_explicit != acquire_point.is_some() {
                    log::warn!("{name}: inconsistent SyncTimeline metadata / SyncObj FD layout");
                    return;
                }
                let acquire_fd = is_explicit.then(|| datas[datas.len() - 2].fd());
                let datas = if is_explicit {
                    &datas[..datas.len() - 2]
                } else {
                    datas
                };
                if datas.is_empty()
                    || datas.len() > 4
                    || datas
                        .iter()
                        .any(|d| d.type_().as_raw() == SPA_DATA_SYNCOBJ)
                {
                    log::warn!("{name}: invalid PipeWire plane layout");
                    return;
                }
                let data_type = datas[0].type_();
                if datas.iter().any(|d| d.type_() != data_type) {
                    log::warn!("{name}: mixed PipeWire data types");
                    return;
                }

                match datas[0].type_() {
                    DataType::DmaBuf => {
                        let mut planes: SmallVec<[FramePlane; 4]> = datas
                            .iter()
                            .map(|p| FramePlane {
                                fd: Some(p.as_raw().fd as _),
                                offset: p.chunk().offset(),
                                stride: p.chunk().stride(),
                            })
                            .collect();

                        let acquire_sync = if is_explicit {
                            let Some(device) = sync_device.as_ref() else {
                                log::error!("{name}: explicit sync without DRM timeline support");
                                return;
                            };
                            let acquire_point = acquire_point.unwrap();
                            let fd = acquire_fd.unwrap();
                            let acquire = match import_timeline(device, fd as RawFd) {
                                Ok(obj) => obj,
                                Err(e) => {
                                    log::error!("{name}: acquire SyncObj import failed: {e}");
                                    return;
                                }
                            };
                            FrameAcquireSync::Explicit {
                                timeline: acquire,
                                point: acquire_point,
                            }
                        } else {
                            let mut fds = Vec::with_capacity(planes.len());
                            for (index, plane) in planes.iter_mut().enumerate() {
                                let fd = plane.fd.unwrap(); // always Some
                                // duplicate the fd so that we can poll it for acquire sync
                                let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
                                if duplicate < 0 {
                                    log::error!(
                                        "{name}: failed to duplicate DMA-buf plane {index} FD {fd}: {}",
                                        std::io::Error::last_os_error()
                                    );
                                    return;
                                }
                                fds.push(unsafe { OwnedFd::from_raw_fd(duplicate) });
                                plane.fd = Some(duplicate);
                            }
                            FrameAcquireSync::Implicit { fds: fds.into() }
                        };

                        let lease_id = held_buffers.borrow_mut().insert(buffer);
                        let tx = tx_ctrl.clone();
                        let lease = FrameLease::new(move || {
                            let _ = tx.send(PwChangeRequest::ReleaseBuffer(lease_id));
                        });

                        let mut dmabuf = DmabufFrame {
                            format: *format,
                            num_planes: planes.len(),
                            planes: Default::default(),
                            mouse: mouse_meta,
                            acquire_sync,
                            lease: Some(lease),
                        };

                        dmabuf.planes[..planes.len()].copy_from_slice(&planes[..planes.len()]);

                        if let Some(result) = receive_callback(&u, WlxFrame::Dmabuf(dmabuf)) {
                            match tx_frame.try_send(result) {
                                Ok(()) | Err(mpsc::TrySendError::Full(_)) => (),
                                Err(mpsc::TrySendError::Disconnected(_)) => {
                                    log::warn!("{name}: frame receiver disconnected");
                                    let _ = stream.disconnect();
                                }
                            }
                        }
                    }
                    DataType::MemFd => {
                        let memfd = MemFdFrame {
                            format: *format,
                            plane: FramePlane {
                                fd: Some(datas[0].as_raw().fd as _),
                                offset: datas[0].chunk().offset(),
                                stride: datas[0].chunk().stride(),
                            },
                            mouse: mouse_meta,
                        };

                        let frame = WlxFrame::MemFd(memfd);
                        if let Some(r) = receive_callback(&u, frame) {
                            match tx_frame.try_send(r) {
                                Ok(_) => (),
                                Err(mpsc::TrySendError::Full(_)) => (),
                                Err(mpsc::TrySendError::Disconnected(_)) => {
                                    log::warn!("{}: disconnected, stopping stream", name);
                                    let _ = stream.disconnect();
                                }
                            }
                        }
                    }
                    DataType::MemPtr => {
                        let raw = datas[0].as_raw();
                        let chunk = datas[0].chunk();
                        let offset = chunk.offset() as usize;
                        let chunk_size = chunk.size() as usize;
                        let Some(size) = offset.checked_add(chunk_size) else {
                            log::error!("{name}: PipeWire chunk range overflow");
                            return;
                        };
                        if size > raw.maxsize as usize {
                            log::error!("{name}: PipeWire chunk extends beyond allocation");
                            return;
                        }
                        let memptr = MemPtrFrame {
                            format: *format,
                            ptr: raw.data as _,
                            size,
                            offset,
                            stride: chunk.stride() as _,
                            mouse: mouse_meta,
                        };

                        let frame = WlxFrame::MemPtr(memptr);
                        if let Some(r) = receive_callback(&u, frame) {
                            match tx_frame.try_send(r) {
                                Ok(_) => (),
                                Err(mpsc::TrySendError::Full(_)) => (),
                                Err(mpsc::TrySendError::Disconnected(_)) => {
                                    log::warn!("{}: disconnected, stopping stream", name);
                                    let _ = stream.disconnect();
                                }
                            }
                        }
                    }
                    _ => log::error!("Received invalid frame data type ({:?})", datas[0].type_()),
                }

                // DMA-buf frames transfer ownership to held_buffers,
                // requeued when their lease expires
            }
        })
        .register()?;

    let mut fourcc_mods: HashMap<DrmFourcc, Vec<DrmModifier>> = HashMap::new();

    for f in dmabuf_formats.iter() {
        if let Some(v) = fourcc_mods.get_mut(&f.code) {
            v.push(f.modifier);
        } else {
            fourcc_mods.insert(f.code, vec![f.modifier]);
        }
    }

    let mut format_params: Vec<Vec<u8>> = fourcc_mods
        .iter()
        .filter_map(|f| obj_to_bytes(get_format_params(Some(f))).ok())
        .collect();

    format_params.push(obj_to_bytes(get_format_params(None)).unwrap()); // safe unwrap: known
    // good values

    let mut params: Vec<&Pod> = format_params
        .iter()
        .filter_map(|bytes| Pod::from_bytes(bytes))
        .collect();

    stream.connect(
        spa::utils::Direction::Input,
        Some(node_id),
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
        params.as_mut_slice(),
    )?;

    let _receiver = rx_ctrl.attach(main_loop.loop_(), {
        let name = name.clone();
        let main_loop = main_loop.clone();
        let held_buffers = held_buffers.clone();

        move |req| {
            log::debug!("{name}: request pipewire stream to {req:?}");
            match req {
                PwChangeRequest::Pause => {
                    let _ = stream
                        .set_active(false)
                        .inspect_err(|e| log::warn!("Could not {req:?} pipewire stream: {e:?}"));
                }
                PwChangeRequest::Resume => {
                    let _ = stream
                        .set_active(true)
                        .inspect_err(|e| log::warn!("Could not {req:?} pipewire stream: {e:?}"));
                }
                PwChangeRequest::ReleaseBuffer(id) => {
                    held_buffers.borrow_mut().requeue(&stream, id);
                }
                PwChangeRequest::Stop => {
                    main_loop.quit();
                }
            }
        }
    });

    main_loop.run();
    log::info!("{}: pipewire loop exited", name);
    Ok::<(), Error>(())
}

fn obj_to_bytes(obj: spa::pod::Object) -> Result<Vec<u8>, GenError> {
    Ok(spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )?
    .0
    .into_inner())
}

fn get_buffer_params() -> Object {
    let data_types = (1 << DataType::MemFd.as_raw())
        | (1 << DataType::MemPtr.as_raw())
        | (1 << DataType::DmaBuf.as_raw());

    get_buffer_params_with_data_types(data_types)
}

fn get_buffer_params_with_data_types(data_types: i32) -> Object {
    // since we hold onto buffers until rendering is done,
    // fewer than 4 buffers may produce stalls
    let buffers = Property {
        key: spa::sys::SPA_PARAM_BUFFERS_buffers,
        flags: PropertyFlags::empty(),
        value: Value::Choice(ChoiceValue::Int(Choice(
            ChoiceFlags::empty(),
            ChoiceEnum::Range {
                default: 6,
                min: 4,
                max: MAX_PW_BUFFERS,
            },
        ))),
    };
    let data_types = Property {
        key: spa::sys::SPA_PARAM_BUFFERS_dataType,
        flags: PropertyFlags::empty(),
        value: Value::Int(data_types),
    };

    spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamBuffers,
        spa::param::ParamType::Buffers,
        buffers,
        data_types,
    )
}

fn get_sync_buffer_params() -> Object {
    let mut obj = get_buffer_params_with_data_types(1 << DataType::DmaBuf.as_raw());
    obj.properties.push(Property {
        key: spa::sys::SPA_PARAM_BUFFERS_blocks,
        flags: PropertyFlags::empty(),
        value: Value::Choice(ChoiceValue::Int(Choice(
            ChoiceFlags::empty(),
            ChoiceEnum::Range {
                default: 3,
                // when using sync timeline, we want at least 1 image plane + 2 syncobj fds
                min: 3,
                // up to 4 image planes
                max: 6,
            },
        ))),
    });
    obj.properties.push(Property {
        key: SPA_PARAM_BUFFERS_META_TYPE,
        flags: PropertyFlags::MANDATORY,
        value: Value::Int(1_i32 << SPA_META_SYNC_TIMELINE),
    });
    obj
}

fn get_meta_object(key: u32, size: usize) -> Object {
    let meta_type_property = Property {
        key: spa::sys::SPA_PARAM_META_type,
        flags: PropertyFlags::empty(),
        value: Value::Id(spa::utils::Id(key)),
    };

    let meta_size_property = Property {
        key: spa::sys::SPA_PARAM_META_size,
        flags: PropertyFlags::empty(),
        value: Value::Int(size as i32),
    };

    spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamMeta,
        spa::param::ParamType::Meta,
        meta_type_property,
        meta_size_property,
    )
}

fn get_format_params(fmt: Option<(&DrmFourcc, &Vec<DrmModifier>)>) -> Object {
    let mut obj = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 256,
                height: 256,
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1,
            },
            spa::utils::Rectangle {
                width: 8192,
                height: 8192,
            }
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction {
                num: 1000,
                denom: 1
            }
        ),
    );

    if let Some(fmt) = fmt {
        let spa_fmt = fourcc_to_spa(*fmt.0);

        let prop = spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa_fmt,
            spa_fmt,
        );
        obj.properties.push(prop);

        // TODO rewrite when property macro supports Long
        let prop = Property {
            key: spa::param::format::FormatProperties::VideoModifier.as_raw(),
            flags: PropertyFlags::MANDATORY | PropertyFlags::DONT_FIXATE,
            value: Value::Choice(ChoiceValue::Long(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Enum {
                    default: u64::from(fmt.1[0]) as _,
                    alternatives: fmt.1.iter().map(|m| u64::from(*m) as _).collect(),
                },
            ))),
        };
        obj.properties.push(prop);
    } else {
        let prop = spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::BGRA,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::ABGR_210LE,
            spa::param::video::VideoFormat::xBGR_210LE,
        );
        obj.properties.push(prop);
    }

    obj
}

fn fourcc_to_spa(fourcc: DrmFourcc) -> VideoFormat {
    match fourcc {
        DrmFourcc::Argb8888 => VideoFormat::BGRA,
        DrmFourcc::Abgr8888 => VideoFormat::RGBA,
        DrmFourcc::Xrgb8888 => VideoFormat::BGRx,
        DrmFourcc::Xbgr8888 => VideoFormat::RGBx,
        DrmFourcc::Abgr2101010 => VideoFormat::ABGR_210LE,
        DrmFourcc::Xbgr2101010 => VideoFormat::xBGR_210LE,
        _ => panic!("Unsupported format"),
    }
}

#[allow(non_upper_case_globals)]
fn spa_to_fourcc(spa: VideoFormat) -> DrmFourcc {
    match spa {
        VideoFormat::BGRA => DrmFourcc::Argb8888,
        VideoFormat::RGBA => DrmFourcc::Abgr8888,
        VideoFormat::BGRx => DrmFourcc::Xrgb8888,
        VideoFormat::RGBx => DrmFourcc::Xbgr8888,
        VideoFormat::ABGR_210LE => DrmFourcc::Abgr2101010,
        VideoFormat::xBGR_210LE => DrmFourcc::Xbgr2101010,
        _ => panic!("Unsupported format"),
    }
}

#[repr(C)]
struct MetaSyncTimeline {
    _flags: u32,
    _padding: u32,
    acquire_point: u64,
    _release_point: u64,
}

impl spa::buffer::meta::Metadata for MetaSyncTimeline {
    const META_TYPE: u32 = SPA_META_SYNC_TIMELINE;
}
