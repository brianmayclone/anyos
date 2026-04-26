#![no_std]
#![allow(dead_code)]

pub mod option {
    pub enum Option<T> {
        None,
        Some(T),
    }

    pub use Option::{None, Some};

    impl<T> Option<T> {
        pub fn is_some(&self) -> bool;
        pub fn is_none(&self) -> bool;
        pub fn as_ref(&self) -> Option<&T>;
        pub fn as_mut(&mut self) -> Option<&mut T>;
        pub fn unwrap(self) -> T;
        pub fn unwrap_or(self, default: T) -> T;
        pub fn unwrap_or_default(self) -> T;
        pub fn or(self, optb: Option<T>) -> Option<T>;
        pub fn ok_or<E>(self, err: E) -> crate::result::Result<T, E>;
        pub fn map<U, F>(self, f: F) -> Option<U>;
        pub fn map_or<U, F>(self, default: U, f: F) -> U;
        pub fn map_or_else<U, D, F>(self, default: D, f: F) -> U;
        pub unsafe fn unwrap_unchecked(self) -> T;
    }
}

pub mod result {
    pub enum Result<T, E> {
        Ok(T),
        Err(E),
    }

    pub use Result::{Err, Ok};

    impl<T, E> Result<T, E> {
        pub fn is_ok(&self) -> bool;
        pub fn is_err(&self) -> bool;
        pub fn ok(self) -> crate::option::Option<T>;
        pub fn err(self) -> crate::option::Option<E>;
        pub fn unwrap(self) -> T;
        pub fn unwrap_or(self, default: T) -> T;
        pub fn unwrap_or_default(self) -> T;
        pub fn map<U, F>(self, op: F) -> Result<U, E>;
        pub fn map_err<F, O>(self, op: O) -> Result<T, F>;
    }
}

pub mod cmp {
    pub enum Ordering {
        Less,
        Equal,
        Greater,
    }

    pub trait PartialEq<Rhs = Self> {
        fn eq(&self, other: &Rhs) -> bool;
        fn ne(&self, other: &Rhs) -> bool;
    }

    pub trait Eq: PartialEq<Self> {}

    pub trait PartialOrd<Rhs = Self>: PartialEq<Rhs> {
        fn partial_cmp(&self, other: &Rhs) -> crate::option::Option<Ordering>;
    }

    pub trait Ord: Eq + PartialOrd<Self> {
        fn cmp(&self, other: &Self) -> Ordering;
    }

    pub struct Reverse<T>(pub T);

    pub fn min<T: Ord>(v1: T, v2: T) -> T;
    pub fn max<T: Ord>(v1: T, v2: T) -> T;
}

pub mod marker {
    pub trait Copy {}
    pub trait Send {}
    pub trait Sync {}
    pub trait Sized {}
    pub trait Unpin {}

    pub struct PhantomData<T>;
}

pub mod clone {
    pub trait Clone {
        fn clone(&self) -> Self;
    }
}

pub mod default {
    pub trait Default {
        fn default() -> Self;
    }
}

pub mod convert {
    pub enum Infallible {}

    pub trait From<T> {
        fn from(value: T) -> Self;
    }

    pub trait Into<T> {
        fn into(self) -> T;
    }

    pub trait AsRef<T> {
        fn as_ref(&self) -> &T;
    }

    pub trait AsMut<T> {
        fn as_mut(&mut self) -> &mut T;
    }
}

pub mod borrow {
    pub trait Borrow<Borrowed> {
        fn borrow(&self) -> &Borrowed;
    }

    pub trait BorrowMut<Borrowed>: Borrow<Borrowed> {
        fn borrow_mut(&mut self) -> &mut Borrowed;
    }
}

pub mod hash {
    pub trait Hasher {
        fn finish(&self) -> u64;
        fn write(&mut self, bytes: &[u8]);
    }

    pub trait Hash {
        fn hash<H: Hasher>(&self, state: &mut H);
    }
}

pub mod ops {
    pub trait Deref {
        type Target;
        fn deref(&self) -> &Self::Target;
    }

    pub trait DerefMut: Deref {
        fn deref_mut(&mut self) -> &mut Self::Target;
    }

    pub trait Index<Idx> {
        type Output;
        fn index(&self, index: Idx) -> &Self::Output;
    }

    pub trait IndexMut<Idx>: Index<Idx> {
        fn index_mut(&mut self, index: Idx) -> &mut Self::Output;
    }

    pub trait Add<Rhs = Self> {
        type Output;
        fn add(self, rhs: Rhs) -> Self::Output;
    }

    pub trait Sub<Rhs = Self> {
        type Output;
        fn sub(self, rhs: Rhs) -> Self::Output;
    }

    pub trait Mul<Rhs = Self> {
        type Output;
        fn mul(self, rhs: Rhs) -> Self::Output;
    }

    pub trait Div<Rhs = Self> {
        type Output;
        fn div(self, rhs: Rhs) -> Self::Output;
    }

    pub trait Rem<Rhs = Self> {
        type Output;
        fn rem(self, rhs: Rhs) -> Self::Output;
    }

    pub trait BitAnd<Rhs = Self> {
        type Output;
        fn bitand(self, rhs: Rhs) -> Self::Output;
    }

    pub trait BitOr<Rhs = Self> {
        type Output;
        fn bitor(self, rhs: Rhs) -> Self::Output;
    }

    pub trait BitXor<Rhs = Self> {
        type Output;
        fn bitxor(self, rhs: Rhs) -> Self::Output;
    }

    pub trait Not {
        type Output;
        fn not(self) -> Self::Output;
    }

    pub enum Bound<T> {
        Included(T),
        Excluded(T),
        Unbounded,
    }

    pub struct Range<Idx> {
        pub start: Idx,
        pub end: Idx,
    }

    pub struct RangeFrom<Idx> {
        pub start: Idx,
    }

    pub struct RangeTo<Idx> {
        pub end: Idx,
    }

    pub struct RangeInclusive<Idx> {
        pub start: Idx,
        pub end: Idx,
    }
}

pub mod iter {
    pub trait Iterator {
        type Item;
        fn next(&mut self) -> crate::option::Option<Self::Item>;
        fn enumerate(self) -> Enumerate<Self>;
        fn collect<B>(self) -> B;
    }

    pub trait IntoIterator {
        type Item;
        type IntoIter;
        fn into_iter(self) -> Self::IntoIter;
    }

    pub trait FromIterator<A> {
        fn from_iter<T>(iter: T) -> Self;
    }

    pub struct Enumerate<I> {
        pub iter: I,
        pub count: usize,
    }

    pub struct Once<T> {
        pub value: crate::option::Option<T>,
    }

    pub fn once<T>(value: T) -> Once<T> {
        Once {
            value: crate::option::Some(value),
        }
    }
}

pub mod mem {
    pub struct ManuallyDrop<T> {
        pub value: T,
    }

    impl<T> ManuallyDrop<T> {
        pub fn new(value: T) -> ManuallyDrop<T>;
    }

    impl<T> crate::ops::Deref for ManuallyDrop<T> {
        type Target = T;
        fn deref(&self) -> &T;
    }

    impl<T> crate::ops::DerefMut for ManuallyDrop<T> {
        fn deref_mut(&mut self) -> &mut T;
    }

    pub union MaybeUninit<T> {
        pub value: T,
    }

    impl<T> MaybeUninit<T> {
        pub const fn new(value: T) -> MaybeUninit<T>;
        pub const fn uninit() -> MaybeUninit<T>;
        pub fn as_ptr(&self) -> *const T;
        pub fn as_mut_ptr(&mut self) -> *mut T;
        pub unsafe fn assume_init(self) -> T;
        pub unsafe fn assume_init_read(&self) -> T;
        pub unsafe fn assume_init_ref(&self) -> &T;
        pub unsafe fn assume_init_mut(&mut self) -> &mut T;
    }

    pub fn size_of<T>() -> usize;
    pub fn size_of_val<T>(val: &T) -> usize;
    pub unsafe fn size_of_val_raw<T>(val: *const T) -> usize;
    pub fn align_of<T>() -> usize;
    pub fn forget<T>(value: T);
    pub unsafe fn zeroed<T>() -> T;
    pub unsafe fn transmute<T, U>(value: T) -> U;
    pub unsafe fn transmute_copy<T, U>(src: &T) -> U;
    pub fn replace<T>(dest: &mut T, src: T) -> T;
    pub unsafe fn take<T>(src: &mut T) -> T;
}

pub mod ptr {
    pub struct NonNull<T> {
        pub pointer: *mut T,
    }

    impl<T> NonNull<T> {
        pub const unsafe fn new_unchecked(ptr: *mut T) -> NonNull<T>;
        pub fn new(ptr: *mut T) -> crate::option::Option<NonNull<T>>;
        pub fn dangling() -> NonNull<T>;
        pub fn as_ptr(self) -> *mut T;
        pub unsafe fn as_ref<'a>(&self) -> &'a T;
        pub unsafe fn as_mut<'a>(&mut self) -> &'a mut T;
        pub fn cast<U>(self) -> NonNull<U>;
    }

    impl<T> crate::convert::From<&mut T> for NonNull<T> {
        fn from(value: &mut T) -> NonNull<T>;
    }

    impl<T> crate::convert::From<&T> for NonNull<T> {
        fn from(value: &T) -> NonNull<T>;
    }

    pub fn null<T>() -> *const T;
    pub fn null_mut<T>() -> *mut T;
    pub unsafe fn read<T>(src: *const T) -> T;
    pub unsafe fn read_unaligned<T>(src: *const T) -> T;
    pub unsafe fn read_volatile<T>(src: *const T) -> T;
    pub unsafe fn write<T>(dst: *mut T, src: T);
    pub unsafe fn write_unaligned<T>(dst: *mut T, src: T);
    pub unsafe fn write_volatile<T>(dst: *mut T, src: T);
    pub unsafe fn write_bytes<T>(dst: *mut T, val: u8, count: usize);
    pub unsafe fn copy_nonoverlapping<T>(src: *const T, dst: *mut T, count: usize);
    pub unsafe fn drop_in_place<T>(to_drop: *mut T);
    pub unsafe fn swap_nonoverlapping<T>(x: *mut T, y: *mut T, count: usize);
    pub fn slice_from_raw_parts_mut<T>(data: *mut T, len: usize) -> *mut [T];
    pub fn from_ref<T>(s: &T) -> *const T;
}

pub mod slice {
    pub struct Iter<'a, T> {
        pub ptr: *const T,
        pub end: *const T,
        pub marker: crate::marker::PhantomData<&'a T>,
    }

    pub struct IterMut<'a, T> {
        pub ptr: *mut T,
        pub end: *mut T,
        pub marker: crate::marker::PhantomData<&'a mut T>,
    }

    pub unsafe fn from_raw_parts<'a, T>(data: *const T, len: usize) -> &'a [T];
    pub unsafe fn from_raw_parts_mut<'a, T>(data: *mut T, len: usize) -> &'a mut [T];
    pub fn from_ref<T>(s: &T) -> &[T];
}

pub mod str {
    pub struct Utf8Error;

    pub fn from_utf8(v: &[u8]) -> crate::result::Result<&str, Utf8Error>;
    pub unsafe fn from_utf8_unchecked(v: &[u8]) -> &str;
}

pub mod fmt {
    pub struct Error;
    pub type Result = crate::result::Result<(), Error>;

    pub struct Arguments;
    pub struct Formatter<'a> {
        pub marker: crate::marker::PhantomData<&'a ()>,
    }

    pub struct DebugTuple<'a, 'b> {
        pub marker: crate::marker::PhantomData<&'a &'b ()>,
    }

    impl<'a> Formatter<'a> {
        pub fn pad(&mut self, s: &str) -> Result;
        pub fn debug_tuple<'b>(&'b mut self, name: &str) -> DebugTuple<'b, 'a>;
    }

    impl<'a, 'b> DebugTuple<'a, 'b> {
        pub fn field<T>(&mut self, value: &T) -> &mut DebugTuple<'a, 'b>;
        pub fn finish(&mut self) -> Result;
    }

    pub trait Debug {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result;
    }

    pub trait Display {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result;
    }

    pub trait Write {
        fn write_str(&mut self, s: &str) -> Result;
        fn write_fmt(&mut self, args: Arguments) -> Result;
    }

    pub fn write(output: &mut dyn Write, args: Arguments) -> Result;
}

pub mod alloc {
    pub struct Layout {
        pub size: usize,
        pub align: usize,
    }

    impl Layout {
        pub fn from_size_align(
            size: usize,
            align: usize,
        ) -> crate::result::Result<Layout, LayoutError>;
        pub unsafe fn from_size_align_unchecked(size: usize, align: usize) -> Layout;
        pub fn size(&self) -> usize;
        pub fn align(&self) -> usize;
    }

    pub struct LayoutError;

    pub unsafe trait GlobalAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8;
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout);
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8;
    }
}

pub mod sync {
    pub mod atomic {
        pub enum Ordering {
            Relaxed,
            Release,
            Acquire,
            AcqRel,
            SeqCst,
        }

        pub struct AtomicBool {
            pub value: bool,
        }

        pub struct AtomicU32 {
            pub value: u32,
        }

        pub struct AtomicUsize {
            pub value: usize,
        }

        impl AtomicBool {
            pub const fn new(value: bool) -> AtomicBool;
            pub fn load(&self, order: Ordering) -> bool;
            pub fn store(&self, value: bool, order: Ordering);
            pub fn swap(&self, value: bool, order: Ordering) -> bool;
            pub fn compare_exchange_weak(
                &self,
                current: bool,
                new: bool,
                success: Ordering,
                failure: Ordering,
            ) -> crate::result::Result<bool, bool>;
        }

        impl AtomicU32 {
            pub const fn new(value: u32) -> AtomicU32;
            pub fn load(&self, order: Ordering) -> u32;
            pub fn store(&self, value: u32, order: Ordering);
            pub fn fetch_add(&self, value: u32, order: Ordering) -> u32;
        }

        impl AtomicUsize {
            pub const fn new(value: usize) -> AtomicUsize;
            pub fn load(&self, order: Ordering) -> usize;
            pub fn store(&self, value: usize, order: Ordering);
            pub fn fetch_add(&self, value: usize, order: Ordering) -> usize;
        }

        pub fn fence(order: Ordering);
        pub fn compiler_fence(order: Ordering);
    }
}

pub mod hint {
    pub fn likely(b: bool) -> bool;
    pub fn unlikely(b: bool) -> bool;
    pub fn spin_loop();
    pub unsafe fn unreachable_unchecked() -> !;
}

pub mod panic {
    pub struct PanicInfo<'a> {
        pub marker: crate::marker::PhantomData<&'a ()>,
    }
}

pub mod cell {
    pub struct UnsafeCell<T> {
        pub value: T,
    }

    impl<T> UnsafeCell<T> {
        pub const fn new(value: T) -> UnsafeCell<T>;
        pub fn get(&self) -> *mut T;
    }

    pub struct Cell<T> {
        pub value: T,
    }

    pub struct RefCell<T> {
        pub value: T,
    }
}

pub mod num {
    pub struct NonZeroUsize {
        pub value: usize,
    }

    impl NonZeroUsize {
        pub fn new(value: usize) -> crate::option::Option<NonZeroUsize>;
        pub fn get(self) -> usize;
        pub fn unwrap(self) -> NonZeroUsize;
    }

    pub struct Wrapping<T>(pub T);
    pub struct Saturating<T>(pub T);
}

pub mod time {
    pub struct Duration {
        pub secs: u64,
        pub nanos: u32,
    }

    impl Duration {
        pub fn from_secs(secs: u64) -> Duration {
            Duration { secs, nanos: 0 }
        }

        pub fn from_millis(ms: u64) -> Duration {
            Duration {
                secs: ms / 1000,
                nanos: ((ms % 1000) as u32) * 1_000_000,
            }
        }

        pub fn as_millis(&self) -> u128 {
            (self.secs as u128) * 1000 + (self.nanos as u128) / 1_000_000
        }
    }
}

pub mod any {
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct TypeId {
        value: u64,
    }

    impl TypeId {
        pub fn of<T: ?Sized>() -> TypeId;
    }

    pub trait Any {}
    pub fn type_name<T>() -> &'static str;
}

pub mod char {
    pub fn from_u32(v: u32) -> crate::option::Option<char>;
}
pub mod f32 {
    impl f32 {
        pub fn from_bits(v: u32) -> f32;
        pub fn to_bits(self) -> u32;
    }

    pub mod consts {
        pub const PI: f32 = 3.1415927;
        pub const E: f32 = 2.7182817;
        pub const LN_2: f32 = 0.6931472;
        pub const LN_10: f32 = 2.3025851;
        pub const LOG2_E: f32 = 1.442695;
        pub const LOG2_10: f32 = 3.321928;
        pub const LOG10_E: f32 = 0.4342945;
        pub const LOG10_2: f32 = 0.30103;
        pub const FRAC_1_PI: f32 = 0.31830987;
        pub const FRAC_2_PI: f32 = 0.63661975;
        pub const FRAC_2_SQRT_PI: f32 = 1.1283792;
        pub const FRAC_1_SQRT_2: f32 = 0.70710677;
        pub const FRAC_PI_2: f32 = 1.5707964;
        pub const FRAC_PI_3: f32 = 1.0471976;
        pub const FRAC_PI_4: f32 = 0.7853982;
        pub const FRAC_PI_6: f32 = 0.5235988;
        pub const FRAC_PI_8: f32 = 0.3926991;
        pub const SQRT_2: f32 = 1.4142135;
    }
}

pub mod f64 {
    impl f64 {
        pub fn from_bits(v: u64) -> f64;
        pub fn to_bits(self) -> u64;
    }

    pub mod consts {
        pub const PI: f64 = 3.141592653589793;
        pub const E: f64 = 2.718281828459045;
        pub const LN_2: f64 = 0.6931471805599453;
        pub const LN_10: f64 = 2.302585092994046;
        pub const LOG2_E: f64 = 1.4426950408889634;
        pub const LOG2_10: f64 = 3.321928094887362;
        pub const LOG10_E: f64 = 0.4342944819032518;
        pub const LOG10_2: f64 = 0.3010299956639812;
        pub const FRAC_1_PI: f64 = 0.3183098861837907;
        pub const FRAC_2_PI: f64 = 0.6366197723675814;
        pub const FRAC_2_SQRT_PI: f64 = 1.1283791670955126;
        pub const FRAC_1_SQRT_2: f64 = 0.7071067811865476;
        pub const FRAC_PI_2: f64 = 1.5707963267948966;
        pub const FRAC_PI_3: f64 = 1.0471975511965979;
        pub const FRAC_PI_4: f64 = 0.7853981633974483;
        pub const FRAC_PI_6: f64 = 0.5235987755982989;
        pub const FRAC_PI_8: f64 = 0.39269908169872414;
        pub const SQRT_2: f64 = 1.4142135623730951;
    }
}

pub mod prelude {
    pub mod v1 {
        pub use crate::clone::Clone;
        pub use crate::cmp::{Eq, Ord, PartialEq, PartialOrd};
        pub use crate::convert::{AsMut, AsRef, From, Into};
        pub use crate::default::Default;
        pub use crate::marker::{Copy, Send, Sized, Sync};
        pub use crate::option::Option::{self, None, Some};
        pub use crate::result::Result::{self, Err, Ok};
    }
}

pub use clone::Clone;
pub use cmp::{Eq, Ord, Ordering, PartialEq, PartialOrd};
pub use convert::{AsMut, AsRef, From, Into};
pub use default::Default;
pub use marker::{Copy, Send, Sized, Sync};
pub use option::Option;
pub use result::Result;
