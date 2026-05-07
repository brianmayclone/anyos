//! Small SLUB-style object caches for fixed-size kernel objects.
//!
//! This sits above the physical buddy allocator and below typed kernel data
//! structures. It is intentionally narrow: objects come from dedicated slabs
//! backed by physical pages, while the general `alloc` heap remains the fallback
//! for variable-size allocations (`Vec`, strings, filesystem buffers, ...).

use crate::memory::physical::{self, FrameAllocPolicy};
use crate::memory::{physmap, FRAME_SIZE};
use crate::sync::spinlock::Spinlock;
use core::marker::PhantomData;
use core::mem;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

#[repr(C)]
struct FreeObject {
    next: *mut FreeObject,
}

/// A fixed-size kernel object cache.
///
/// Metadata is deliberately tiny and kept in the cache object itself. Free-list
/// links live in freed objects, matching the usual slab/slub shape.
pub struct KmemCache {
    name: &'static str,
    object_size: usize,
    object_align: usize,
    slot_size: usize,
    pages_per_slab: usize,
    objects_per_slab: usize,
    free_list: *mut FreeObject,
    slabs_allocated: usize,
    objects_total: usize,
    objects_free: usize,
}

unsafe impl Send for KmemCache {}

impl KmemCache {
    pub const fn new(name: &'static str, object_size: usize, object_align: usize) -> Self {
        KmemCache {
            name,
            object_size,
            object_align,
            slot_size: 0,
            pages_per_slab: 0,
            objects_per_slab: 0,
            free_list: core::ptr::null_mut(),
            slabs_allocated: 0,
            objects_total: 0,
            objects_free: 0,
        }
    }

    #[inline]
    fn align_up(value: usize, align: usize) -> usize {
        debug_assert!(align.is_power_of_two());
        (value + align - 1) & !(align - 1)
    }

    fn ensure_layout(&mut self) -> bool {
        if self.slot_size != 0 {
            return true;
        }
        let align = self.object_align.max(mem::align_of::<FreeObject>());
        if !align.is_power_of_two() || align > FRAME_SIZE {
            return false;
        }
        let min_size = self.object_size.max(mem::size_of::<FreeObject>());
        let slot_size = Self::align_up(min_size, align);
        let pages = (slot_size + FRAME_SIZE - 1) / FRAME_SIZE;
        let pages = pages.max(1);
        let slab_bytes = pages * FRAME_SIZE;
        let objects = (slab_bytes / slot_size).max(1);

        self.object_align = align;
        self.slot_size = slot_size;
        self.pages_per_slab = pages;
        self.objects_per_slab = objects;
        true
    }

    fn grow(&mut self) -> bool {
        if !self.ensure_layout() {
            return false;
        }

        let phys = match physical::alloc_contiguous_with(self.pages_per_slab, FrameAllocPolicy::Any)
        {
            Some(phys) => phys,
            None => return false,
        };
        let slab = match physmap::phys_to_virt(phys) {
            Some(ptr) => ptr as usize,
            None => {
                for page in 0..self.pages_per_slab {
                    physical::free_frame(crate::memory::address::PhysAddr::new(
                        phys.as_u64() + (page * FRAME_SIZE) as u64,
                    ));
                }
                return false;
            }
        };

        let start = Self::align_up(slab, self.object_align);
        for i in 0..self.objects_per_slab {
            let obj = (start + i * self.slot_size) as *mut FreeObject;
            unsafe {
                (*obj).next = self.free_list;
            }
            self.free_list = obj;
        }

        self.slabs_allocated += 1;
        self.objects_total += self.objects_per_slab;
        self.objects_free += self.objects_per_slab;
        true
    }

    pub fn alloc_raw(&mut self) -> *mut u8 {
        if self.free_list.is_null() && !self.grow() {
            return core::ptr::null_mut();
        }

        let obj = self.free_list;
        if obj.is_null() {
            return core::ptr::null_mut();
        }
        unsafe {
            self.free_list = (*obj).next;
        }
        self.objects_free = self.objects_free.saturating_sub(1);
        obj as *mut u8
    }

    /// # Safety
    /// `ptr` must have been returned by this cache and must not currently be
    /// live. The object's destructor must already have run.
    pub unsafe fn free_raw(&mut self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        let obj = ptr as *mut FreeObject;
        (*obj).next = self.free_list;
        self.free_list = obj;
        self.objects_free += 1;
    }

    pub fn stats(&self) -> SlabStats {
        SlabStats {
            name: self.name,
            object_size: self.object_size,
            slot_size: self.slot_size,
            pages_per_slab: self.pages_per_slab,
            slabs_allocated: self.slabs_allocated,
            objects_total: self.objects_total,
            objects_free: self.objects_free,
        }
    }
}

#[derive(Clone, Copy)]
pub struct SlabStats {
    pub name: &'static str,
    pub object_size: usize,
    pub slot_size: usize,
    pub pages_per_slab: usize,
    pub slabs_allocated: usize,
    pub objects_total: usize,
    pub objects_free: usize,
}

/// Owned object allocated from a [`KmemCache`].
pub struct KBox<T: 'static> {
    ptr: NonNull<T>,
    cache: &'static Spinlock<KmemCache>,
    _marker: PhantomData<T>,
}

unsafe impl<T: Send> Send for KBox<T> {}
unsafe impl<T: Sync> Sync for KBox<T> {}

impl<T> KBox<T> {
    pub fn new_in(value: T, cache: &'static Spinlock<KmemCache>) -> Option<Self> {
        let raw = {
            let mut guard = cache.lock();
            guard.alloc_raw()
        } as *mut T;

        let ptr = NonNull::new(raw)?;
        unsafe {
            ptr.as_ptr().write(value);
        }

        Some(KBox {
            ptr,
            cache,
            _marker: PhantomData,
        })
    }

    pub fn as_ref(&self) -> &T {
        self
    }

    pub fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T> Deref for KBox<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> DerefMut for KBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

impl<T> Drop for KBox<T> {
    fn drop(&mut self) {
        unsafe {
            core::ptr::drop_in_place(self.ptr.as_ptr());
        }
        let mut guard = self.cache.lock();
        unsafe {
            guard.free_raw(self.ptr.as_ptr() as *mut u8);
        }
    }
}
