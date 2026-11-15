use std::{marker::PhantomData, mem, ptr::NonNull};

use pipewire::{
    self as pw,
    spa::{self, buffer::meta::Metadata},
    stream::Stream,
};

pub(super) struct DequeuedBuffer<'a> {
    stream: &'a Stream,
    ptr: NonNull<pw::sys::pw_buffer>,
    _not_send: PhantomData<std::rc::Rc<()>>,
}

impl<'a> DequeuedBuffer<'a> {
    pub(super) fn dequeue(stream: &'a Stream) -> Option<Self> {
        let ptr = NonNull::new(unsafe { stream.dequeue_raw_buffer() })?;
        Some(Self {
            stream,
            ptr,
            _not_send: PhantomData,
        })
    }

    fn spa(&self) -> Option<NonNull<spa::sys::spa_buffer>> {
        NonNull::new(unsafe { self.ptr.read().buffer })
    }

    pub(super) fn datas_mut(&self) -> &'a mut [pw::spa::buffer::Data] {
        let Some(buffer) = self.spa() else {
            return &mut [];
        };
        unsafe {
            let n_datas = buffer.read().n_datas;
            if n_datas > 0
                && let Some(datas) = NonNull::new(buffer.read().datas)
            {
                let datas = datas.as_ptr() as *mut pw::spa::buffer::Data;
                std::slice::from_raw_parts_mut(datas, usize::try_from(n_datas).unwrap())
            } else {
                &mut []
            }
        }
    }

    pub(super) fn find_meta<T>(&self) -> Option<&'a T>
    where
        T: Metadata,
    {
        let buffer = self.spa()?;
        unsafe {
            if buffer.read().n_metas != 0 {
                match T::META_TYPE {
                    pw::spa::sys::SPA_META_VideoDamage => {
                        let meta_data =
                            pw::spa::sys::spa_buffer_find_meta(buffer.as_ptr(), T::META_TYPE)
                                as *const T;
                        if !meta_data.is_null() {
                            return Some(&*meta_data);
                        }
                    }
                    _ => {
                        let meta_data = pw::spa::sys::spa_buffer_find_meta_data(
                            buffer.as_ptr(),
                            T::META_TYPE,
                            std::mem::size_of::<T>(),
                        ) as *const T;
                        if !meta_data.is_null() {
                            return Some(&*meta_data);
                        }
                    }
                }
            }
        }
        None
    }
}

pub(super) const MAX_PW_BUFFERS: i32 = 8;

impl Drop for DequeuedBuffer<'_> {
    fn drop(&mut self) {
        unsafe { self.stream.queue_raw_buffer(self.ptr.as_ptr()) };
    }
}

#[derive(Default)]
pub(super) struct HeldBuffers {
    buffers: Vec<NonNull<pw::sys::pw_buffer>>,
}

impl HeldBuffers {
    pub(super) fn insert<'a>(&mut self, buffer: DequeuedBuffer<'a>) -> usize {
        self.buffers.push(buffer.ptr);
        let ret_val = buffer.ptr.as_ptr() as usize;
        mem::forget(buffer);
        ret_val
    }

    pub(super) fn on_removed(&mut self, buffer: NonNull<pw::sys::pw_buffer>) -> bool {
        let before_count = self.buffers.len();
        self.buffers.retain(|ptr| ptr.as_ptr() != buffer.as_ptr());
        before_count != self.buffers.len()
    }

    pub(super) fn requeue(&mut self, stream: &pw::stream::Stream, id: usize) {
        self.buffers.retain(|ptr| {
            if ptr.as_ptr() as usize == id {
                unsafe { stream.queue_raw_buffer(ptr.as_ptr()) };
                false
            } else {
                true
            }
        });
    }
}

impl Drop for HeldBuffers {
    fn drop(&mut self) {
        log::warn!("{} checked-out buffers at shutdown", self.buffers.len());
        self.buffers.clear();
    }
}
