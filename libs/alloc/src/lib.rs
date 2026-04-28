#![no_std]
#![allow(dead_code)]

pub mod alloc {
    pub use core::alloc::{GlobalAlloc, Layout, LayoutError};

    pub struct AllocError;

    pub struct Global;

    pub unsafe trait Allocator {
        fn allocate(&self, layout: Layout) -> core::result::Result<core::ptr::NonNull<u8>, AllocError>;
        unsafe fn deallocate(&self, ptr: core::ptr::NonNull<u8>, layout: Layout);
    }

    unsafe impl Allocator for Global {
        fn allocate(&self, layout: Layout) -> core::result::Result<core::ptr::NonNull<u8>, AllocError>;
        unsafe fn deallocate(&self, ptr: core::ptr::NonNull<u8>, layout: Layout);
    }

    impl core::default::Default for Global {
        fn default() -> Global;
    }

    pub unsafe fn alloc(layout: Layout) -> *mut u8;
    pub unsafe fn dealloc(ptr: *mut u8, layout: Layout);
    pub unsafe fn realloc(ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8;
    pub fn handle_alloc_error(layout: Layout) -> !;
}

pub mod boxed {
    pub struct Box<T> {
        pub ptr: *mut T,
    }

    impl<T> Box<T> {
        pub fn new(value: T) -> Box<T>;
        pub fn leak<'a>(boxed: Box<T>) -> &'a mut T;
        pub fn as_ref(&self) -> &T;
    }

    impl<T> core::ops::Deref for Box<T> {
        type Target = T;
        fn deref(&self) -> &T;
    }

    impl<T> core::ops::DerefMut for Box<T> {
        fn deref_mut(&mut self) -> &mut T;
    }
}

pub mod vec {
    pub struct Vec<T> {
        pub ptr: *mut T,
        pub len: usize,
        pub cap: usize,
    }

    pub struct IntoIter<T> {
        pub ptr: *mut T,
        pub len: usize,
        pub index: usize,
    }

    impl<T> Vec<T> {
        pub fn new() -> Vec<T>;
        pub fn with_capacity(capacity: usize) -> Vec<T>;
        pub unsafe fn from_raw_parts(ptr: *mut T, length: usize, capacity: usize) -> Vec<T>;
        pub fn len(&self) -> usize;
        pub fn capacity(&self) -> usize;
        pub fn is_empty(&self) -> bool;
        pub fn reserve(&mut self, additional: usize);
        pub fn push(&mut self, value: T);
        pub fn pop(&mut self) -> core::option::Option<T>;
        pub fn clear(&mut self);
        pub fn retain<F>(&mut self, f: F);
        pub fn drain<R>(&mut self, range: R) -> IntoIter<T>;
        pub fn resize(&mut self, new_len: usize, value: T);
        pub fn remove(&mut self, index: usize) -> T;
        pub fn get<I>(&self, index: I) -> core::option::Option<&T>;
        pub fn truncate(&mut self, len: usize);
        pub fn last(&self) -> core::option::Option<&T>;
        pub fn as_ptr(&self) -> *const T;
        pub fn as_mut_ptr(&mut self) -> *mut T;
        pub fn as_slice(&self) -> &[T];
        pub fn as_mut_slice(&mut self) -> &mut [T];
        pub fn into_iter(self) -> IntoIter<T>;
        pub fn sort_by<F>(&mut self, compare: F);
        pub fn sort(&mut self);
        pub fn sort_by_key<K, F>(&mut self, f: F);
        pub fn dedup_by<F>(&mut self, same_bucket: F);
        pub fn last_mut(&mut self) -> core::option::Option<&mut T>;
        pub fn copy_from_slice(&mut self, src: &[T]);
        pub fn join(&self, sep: &str) -> crate::string::String;
    }
}

pub mod string {
    pub struct String {
        pub ptr: *mut u8,
        pub len: usize,
        pub cap: usize,
    }

    pub trait ToString {
        fn to_string(&self) -> String;
    }

    impl String {
        pub fn new() -> String;
        pub fn with_capacity(capacity: usize) -> String;
        pub fn from(s: &str) -> String;
        pub fn len(&self) -> usize;
        pub fn is_empty(&self) -> bool;
        pub fn clear(&mut self);
        pub fn push_str(&mut self, string: &str);
        pub fn push(&mut self, ch: char);
        pub fn as_str(&self) -> &str;
        pub fn as_bytes(&self) -> &[u8];
        pub fn cmp(&self, other: &String) -> core::cmp::Ordering;
        pub fn find(&self, pat: &str) -> core::option::Option<usize>;
        pub fn rfind(&self, pat: &str) -> core::option::Option<usize>;
        pub fn remove(&mut self, idx: usize) -> char;
        pub fn into_bytes(self) -> crate::vec::Vec<u8>;
        pub fn from_utf8(vec: crate::vec::Vec<u8>) -> core::result::Result<String, FromUtf8Error>;
        pub fn from_utf8_lossy(v: &[u8]) -> crate::borrow::Cow<'_, str>;
    }

    pub struct FromUtf8Error;

    impl FromUtf8Error {
        pub fn into_bytes(self) -> crate::vec::Vec<u8>;
    }
}

pub mod borrow {
    pub use core::borrow::{Borrow, BorrowMut};

    pub trait ToOwned {
        type Owned;
        fn to_owned(&self) -> Self::Owned;
    }

    pub enum Cow<'a, B: ?Sized + ToOwned + 'a> {
        Borrowed(&'a B),
        Owned(<B as ToOwned>::Owned),
    }

    impl<'a, B: ?Sized + ToOwned + 'a> Cow<'a, B> {
        pub fn is_borrowed(&self) -> bool;
        pub fn is_owned(&self) -> bool;
        pub fn into_owned(self) -> <B as ToOwned>::Owned;
    }

    impl<'a, B: ?Sized + ToOwned + 'a> core::ops::Deref for Cow<'a, B> {
        type Target = B;
        fn deref(&self) -> &Self::Target;
    }
}

pub mod collections {
    pub struct VecDeque<T> {
        pub vec: crate::vec::Vec<T>,
    }

    impl<T> VecDeque<T> {
        pub fn new() -> VecDeque<T>;
        pub fn with_capacity(capacity: usize) -> VecDeque<T>;
        pub fn push_back(&mut self, value: T);
        pub fn pop_front(&mut self) -> core::option::Option<T>;
        pub fn clear(&mut self);
        pub fn reserve(&mut self, additional: usize);
        pub fn len(&self) -> usize;
        pub fn is_empty(&self) -> bool;
    }

    pub struct BTreeMap<K, V> {
        pub marker: core::marker::PhantomData<(K, V)>,
    }

    pub struct BTreeSet<T> {
        pub marker: core::marker::PhantomData<T>,
    }

    pub struct BinaryHeap<T> {
        pub vec: crate::vec::Vec<T>,
    }

    pub struct LinkedList<T> {
        pub marker: core::marker::PhantomData<T>,
    }

    impl<T> LinkedList<T> {
        pub fn new() -> LinkedList<T>;
        pub fn push_back(&mut self, elt: T);
        pub fn clear(&mut self);
    }

    pub mod btree_map {
        pub struct Iter<'a, K, V> {
            pub marker: core::marker::PhantomData<&'a (K, V)>,
        }
    }

    pub mod btree_set {
        pub struct Iter<'a, T> {
            pub marker: core::marker::PhantomData<&'a T>,
        }
    }
}

pub mod rc {
    pub struct Rc<T> {
        pub ptr: *mut T,
    }

    pub struct Weak<T> {
        pub ptr: *mut T,
    }

    impl<T> Rc<T> {
        pub fn new(value: T) -> Rc<T>;
        pub fn downgrade(this: &Rc<T>) -> Weak<T>;
        pub fn as_ptr(this: &Rc<T>) -> *const T;
        pub fn into_raw(this: Rc<T>) -> *const T;
        pub fn strong_count(_this: &Rc<T>) -> usize {
            1
        }
    }
}

pub mod sync {
    pub struct Arc<T> {
        pub ptr: *mut T,
    }

    pub struct Weak<T> {
        pub ptr: *mut T,
    }

    impl<T> Arc<T> {
        pub fn new(value: T) -> Arc<T>;
        pub fn strong_count(_this: &Arc<T>) -> usize {
            1
        }
    }

    impl<T> core::ops::Deref for Arc<T> {
        type Target = T;
        fn deref(&self) -> &Self::Target;
    }
}

pub mod ffi {
    pub struct CString {
        pub bytes: crate::vec::Vec<u8>,
    }

    pub struct CStr;
    pub struct BoxedCStr {
        pub ptr: *mut CStr,
    }
    pub struct NulError;
    pub struct FromBytesWithNulError;

    impl CString {
        pub fn into_boxed_c_str(self) -> BoxedCStr;
    }
}

pub mod format {
    pub fn format(args: core::fmt::Arguments) -> crate::string::String;
}

pub use boxed::Box;
pub use string::String;
pub use vec::Vec;
