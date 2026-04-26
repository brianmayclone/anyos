#![no_std]
#![allow(dead_code)]

pub mod boxed {
    pub struct Box<T> {
        pub ptr: *mut T,
    }

    impl<T> Box<T> {
        pub fn new(value: T) -> Box<T>;
        pub fn leak<'a>(boxed: Box<T>) -> &'a mut T;
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
        pub fn push(&mut self, value: T);
        pub fn pop(&mut self) -> core::option::Option<T>;
        pub fn clear(&mut self);
        pub fn truncate(&mut self, len: usize);
        pub fn last(&self) -> core::option::Option<&T>;
        pub fn as_ptr(&self) -> *const T;
        pub fn as_mut_ptr(&mut self) -> *mut T;
        pub fn as_slice(&self) -> &[T];
        pub fn as_mut_slice(&mut self) -> &mut [T];
        pub fn into_iter(self) -> IntoIter<T>;
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
        pub fn into_bytes(self) -> crate::vec::Vec<u8>;
    }
}

pub mod borrow {
    pub use core::borrow::{Borrow, BorrowMut};

    pub trait ToOwned {
        type Owned;
        fn to_owned(&self) -> Self::Owned;
    }
}

pub mod collections {
    pub struct VecDeque<T> {
        pub vec: crate::vec::Vec<T>,
    }

    impl<T> VecDeque<T> {
        pub fn new() -> VecDeque<T>;
        pub fn push_back(&mut self, value: T);
        pub fn pop_front(&mut self) -> core::option::Option<T>;
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

    impl<T> Rc<T> {
        pub fn new(value: T) -> Rc<T>;
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
    }
}

pub mod ffi {
    pub struct CString {
        pub bytes: crate::vec::Vec<u8>,
    }

    pub struct NulError;
    pub struct FromBytesWithNulError;
}

pub mod format {
    pub fn format(args: core::fmt::Arguments) -> crate::string::String;
}

pub use boxed::Box;
pub use string::String;
pub use vec::Vec;
