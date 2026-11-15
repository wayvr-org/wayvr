use std::{
    fs, io,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, RawFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    sync::Arc,
};

pub use timeline_syncobj::render_node::DrmRenderNode;
use timeline_syncobj::timeline_syncobj::TimelineSyncObj;

const DRM_CAP_SYNCOBJ: u64 = 0x13;
const DRM_CAP_SYNCOBJ_TIMELINE: u64 = 0x14;

#[repr(C)]
struct DrmGetCap {
    capability: u64,
    value: u64,
}

const DRM_IOCTL_GET_CAP: libc::c_ulong = ((3_u64 << 30)
    | ((std::mem::size_of::<DrmGetCap>() as u64) << 16)
    | ((b'd' as u64) << 8)
    | 0x0c) as libc::c_ulong;

fn get_cap(node: &DrmRenderNode, capability: u64) -> io::Result<u64> {
    let mut arg = DrmGetCap {
        capability,
        value: 0,
    };
    let ret = unsafe { libc::ioctl(node.as_fd().as_raw_fd(), DRM_IOCTL_GET_CAP, &mut arg) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(arg.value)
    }
}

fn supports_timeline(node: &DrmRenderNode) -> bool {
    [DRM_CAP_SYNCOBJ, DRM_CAP_SYNCOBJ_TIMELINE]
        .into_iter()
        .all(|cap| get_cap(node, cap).is_ok_and(|value| value != 0))
}

pub fn open_render_node(render_major: u32, render_minor: u32) -> io::Result<DrmRenderNode> {
    if !(128..=255).contains(&render_minor) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("Unsupported DRM render minor: {render_minor}"),
        ));
    }

    let path = format!("/dev/dri/renderD{render_minor}");
    let metadata = fs::metadata(&path)?;

    // verify that device actually matches the provided minor/major
    if !metadata.file_type().is_char_device()
        || metadata.rdev() != libc::makedev(render_major, render_minor)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{path} does not match DRM device {render_major}:{render_minor}"),
        ));
    }

    let node = DrmRenderNode::new(render_minor.into())
        .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))?;

    if !supports_timeline(&node) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{path} does not support DRM syncobj timelines"),
        ));
    }

    Ok(node)
}

pub(crate) fn import_timeline(node: &DrmRenderNode, fd: RawFd) -> io::Result<Arc<TimelineSyncObj>> {
    if fd < 0 {
        return Err(io::Error::from_raw_os_error(libc::EBADF));
    }
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
    TimelineSyncObj::import(node, fd)
        .map(Arc::new)
        .map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))
}
