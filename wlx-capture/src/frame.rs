use std::{
    os::fd::{AsRawFd, OwnedFd, RawFd},
    sync::Arc,
};

#[cfg(feature = "pipewire")]
use timeline_syncobj::timeline_syncobj::TimelineSyncObj;

use drm_fourcc::{DrmFormat, DrmFourcc, DrmModifier};

#[cfg(feature = "egl")]
#[rustfmt::skip]
const EGL_DMABUF_PLANE_ATTRS: [isize; 20] = [
//  FD     Offset Stride ModLo  ModHi
    0x3272,0x3273,0x3274,0x3443,0x3444,
    0x3275,0x3276,0x3277,0x3445,0x3446,
    0x3278,0x3279,0x327A,0x3447,0x3448,
    0x3440,0x3441,0x3442,0x3449,0x344A,
];

pub enum WlxFrame {
    Dmabuf(DmabufFrame),
    MemFd(MemFdFrame),
    MemPtr(MemPtrFrame),
    Implicit(Transform),
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Transform {
    #[default]
    Undefined,
    Normal,
    Rotated90,
    Rotated180,
    Rotated270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

#[derive(Debug, Clone, Copy)]
pub struct FrameFormat {
    pub width: u32,
    pub height: u32,
    pub drm_format: DrmFormat,
    pub transform: Transform,
}

impl FrameFormat {
    #[must_use]
    pub fn get_mod_hi(&self) -> u32 {
        let m = u64::from(self.drm_format.modifier);
        (m >> 32) as _
    }
    #[must_use]
    pub fn get_mod_lo(&self) -> u32 {
        let m = u64::from(self.drm_format.modifier);
        (m & 0xFFFFFFFF) as _
    }
    pub fn set_mod(&mut self, mod_hi: u32, mod_low: u32) {
        self.drm_format.modifier = DrmModifier::from(((mod_hi as u64) << 32) + mod_low as u64);
    }
}

#[derive(Clone, Copy, Default)]
pub struct FramePlane {
    pub fd: Option<RawFd>,
    pub offset: u32,
    pub stride: i32,
}

pub struct DmabufFrame {
    pub format: FrameFormat,
    pub num_planes: usize,
    pub planes: [FramePlane; 4],
    pub mouse: Option<MouseMeta>,
    pub acquire_sync: FrameAcquireSync,
    pub lease: Option<FrameLease>,
}

impl DmabufFrame {
    #[cfg(feature = "egl")]
    /// Get the attributes for creating an EGLImage.
    /// Pacics if fd is None; check using `is_valid` first.
    pub fn get_egl_image_attribs(&self) -> Vec<isize> {
        let mut vec: Vec<isize> = vec![
            0x3057, // WIDTH
            self.format.width as _,
            0x3056, // HEIGHT
            self.format.height as _,
            0x3271, // LINUX_DRM_FOURCC_EXT,
            self.format.drm_format.code as _,
        ];

        for i in 0..self.num_planes {
            let mut a = i * 5usize;
            vec.push(EGL_DMABUF_PLANE_ATTRS[a]);
            vec.push(self.planes[i].fd.unwrap() as _); // safe to unwrap due to contract
            a += 1;
            vec.push(EGL_DMABUF_PLANE_ATTRS[a]);
            vec.push(self.planes[i].offset as _);
            a += 1;
            vec.push(EGL_DMABUF_PLANE_ATTRS[a]);
            vec.push(self.planes[i].stride as _);
            a += 1;
            vec.push(EGL_DMABUF_PLANE_ATTRS[a]);
            vec.push(self.format.get_mod_lo() as _);
            a += 1;
            vec.push(EGL_DMABUF_PLANE_ATTRS[a]);
            vec.push(self.format.get_mod_hi() as _);
        }
        vec.push(0x3038); // NONE

        vec
    }

    /// Returns true if all planes have a valid file descriptor.
    pub fn is_valid(&self) -> bool {
        for i in 0..self.num_planes {
            if self.planes[i].fd.is_none() {
                return false;
            }
        }
        true
    }
}

pub struct MemFdFrame {
    pub format: FrameFormat,
    pub plane: FramePlane,
    pub mouse: Option<MouseMeta>,
}

pub struct MemPtrFrame {
    pub format: FrameFormat,
    pub ptr: usize,
    pub size: usize,
    pub offset: usize,
    pub stride: usize,
    pub mouse: Option<MouseMeta>,
}

#[derive(Default, Clone, PartialEq)]
pub struct MouseMeta {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone)]
pub struct FrameLease(Arc<FrameLeaseInner>);

struct FrameLeaseInner(Box<dyn Fn() + Send + Sync>);

impl FrameLease {
    pub fn new(f: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(FrameLeaseInner(Box::new(f))))
    }
}

impl Drop for FrameLeaseInner {
    fn drop(&mut self) {
        (self.0)();
    }
}

#[derive(Clone, Default)]
pub enum FrameAcquireSync {
    #[default]
    /// Frame is ready, no sync needed
    Ready,
    /// Implicit sync using polled FDs
    Implicit { fds: Arc<[OwnedFd]> },
    #[cfg(feature = "pipewire")]
    /// Explicit sync using DRM SyncObj
    Explicit {
        timeline: Arc<TimelineSyncObj>,
        point: u64,
    },
}

impl FrameAcquireSync {
    pub fn check_ready(&mut self) -> std::io::Result<bool> {
        match self {
            FrameAcquireSync::Ready => Ok(true),
            FrameAcquireSync::Implicit { fds } => {
                if fds.is_empty() {
                    return Err(std::io::Error::other("implicit DMA-buf without planes"));
                }
                for plane_fd in fds.iter() {
                    let fd = plane_fd.as_raw_fd();
                    let mut pfd = libc::pollfd {
                        fd,
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    let result = unsafe { libc::poll(&mut pfd, 1, 0) };
                    if result < 0 {
                        let error = std::io::Error::last_os_error();
                        if error.kind() == std::io::ErrorKind::Interrupted {
                            return Ok(false);
                        }
                        return Err(error);
                    }
                    if result == 0 {
                        return Ok(false);
                    }
                    if pfd.revents & (libc::POLLNVAL | libc::POLLHUP) != 0 {
                        return Err(std::io::Error::other(format!(
                            "DMA-buf plane fd {fd} poll returned with {:#x}",
                            pfd.revents
                        )));
                    }
                    if pfd.revents & libc::POLLERR != 0 {
                        static WARNED: std::sync::atomic::AtomicBool =
                            std::sync::atomic::AtomicBool::new(false);
                        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                            log::error!(
                                "Could not poll implicit acquire fence. Falling back to unsynchronized behavior. Visual artifacts may happen."
                            );
                        }
                        continue;
                    }
                    if pfd.revents & libc::POLLIN == 0 {
                        return Ok(false);
                    }
                }
                *self = FrameAcquireSync::Ready;
                Ok(true)
            }
            #[cfg(feature = "pipewire")]
            FrameAcquireSync::Explicit { timeline, point } => {
                if *point == 0 {
                    return Ok(true);
                }
                if timeline
                    .query_signaled_point()
                    .map(|signaled| signaled >= *point)
                    .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?
                {
                    *self = FrameAcquireSync::Ready;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }
}

pub trait DmaExporter {
    fn next_frame(
        &mut self,
        width: u32,
        height: u32,
        fourcc: DrmFourcc,
    ) -> Option<(FramePlane, DrmModifier)>;
}
