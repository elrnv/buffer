//! This module defines the function pointers for supported traits from the standard library.
//!
//! `CloneFromFn` and `DropFn` enable the use of `VecClone`.
//!
//! The remaining traits improve compatibility with the rest of the standard library.

use crate::bytes::*;
use std::hash::{Hash, Hasher};
use std::mem::ManuallyDrop;
use std::fmt;

pub trait CloneBytes: Clone {
    unsafe fn clone_bytes(src: &[u8]) -> Box<[u8]>;
    unsafe fn clone_from_bytes(dst: &mut [u8], src: &[u8]);
}

pub trait DropBytes {
    unsafe fn drop_bytes(bytes: &mut [u8]);
}

pub trait PartialEqBytes: PartialEq {
    unsafe fn eq_bytes(a: &[u8], b: &[u8]) -> bool;
}

pub trait EqBytes: PartialEqBytes + Eq { }

pub trait HashBytes: Hash {
    unsafe fn hash_bytes(bytes: &[u8], state: &mut dyn Hasher);
}

pub trait DebugBytes: fmt::Debug {
    unsafe fn fmt_bytes(bytes: &[u8], f: &mut fmt::Formatter) -> Result<(), fmt::Error>;
}

impl<T: Clone + Bytes> CloneBytes for T {
    #[inline]
    unsafe fn clone_bytes(src: &[u8]) -> Box<[u8]> {
        let typed_src: &T = Bytes::from_bytes(src);
        Bytes::box_into_box_bytes(Box::new(typed_src.clone()))
    }
    #[inline]
    unsafe fn clone_from_bytes(dst: &mut [u8], src: &[u8]) {
        let typed_src: &T = Bytes::from_bytes(src);
        let typed_dst: &mut T = Bytes::from_bytes_mut(dst);
        typed_dst.clone_from(typed_src);
    }
}

impl<T: Bytes> DropBytes for T {
    #[inline]
    unsafe fn drop_bytes(bytes: &mut [u8]) {
        let md: &mut ManuallyDrop<T> = Bytes::from_bytes_mut(bytes);
        ManuallyDrop::drop(md);
    }
}

impl<T: Bytes + PartialEq> PartialEqBytes for T {
    #[inline]
    unsafe fn eq_bytes(a: &[u8], b: &[u8]) -> bool {
        let (a, b): (&T, &T) = (Bytes::from_bytes(a), Bytes::from_bytes(b));
        a.eq(b)
    }
}

impl<T: Bytes + PartialEqBytes + Eq> EqBytes for T { }

impl<T: Bytes + Hash> HashBytes for T {
    #[inline]
    unsafe fn hash_bytes(bytes: &[u8], mut state: &mut dyn Hasher) {
        let typed_data: &T = Bytes::from_bytes(bytes);
        typed_data.hash(&mut state)
    }
}

impl<T: Bytes + fmt::Debug> DebugBytes for T {
    #[inline]
    unsafe fn fmt_bytes(bytes: &[u8], f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let typed_data: &T = Bytes::from_bytes(bytes);
        typed_data.fmt(f)
    }
}

pub(crate) type CloneFnType = unsafe fn(&[u8]) -> Box<[u8]>;
pub(crate) type CloneFromFnType = unsafe fn(&mut [u8], &[u8]);
pub(crate) type DropFnType = unsafe fn(&mut [u8]);
pub(crate) type EqFnType = unsafe fn(&[u8], &[u8]) -> bool;
pub(crate) type HashFnType = unsafe fn(&[u8], &mut dyn Hasher);
pub(crate) type FmtFnType = unsafe fn (&[u8], &mut fmt::Formatter) -> Result<(), fmt::Error>;

macro_rules! impl_fn_wrapper {
    (derive() struct $fn:ident ( $fn_type:ident )) => {
        pub struct $fn (pub(crate) $fn_type);

        impl_fn_wrapper!(@impls $fn ( $fn_type ));
    };
    ($derives:meta struct $fn:ident ( $fn_type:ident )) => {
        #[$derives]
        pub struct $fn (pub(crate) $fn_type);

        impl_fn_wrapper!(@impls $fn ( $fn_type ));
    };
    (@impls $fn:ident ( $fn_type:ident )) => {
        impl $fn {
            pub fn new(f: $fn_type) -> Self {
                $fn(f)
            }
        }

        impl fmt::Debug for $fn {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($fn)).finish()
            }
        }

        impl PartialEq for $fn {
            fn eq(&self, _: &Self) -> bool {
                // Equality is completely determined by VecCopy.
                true
            }
        }

        impl Hash for $fn {
            fn hash<H: Hasher>(&self, state: &mut H) {
                (self.0 as usize).hash(state);
            }
        }
    }
}

impl_fn_wrapper!(derive(Copy, Clone) struct CloneFn(CloneFnType));
impl_fn_wrapper!(derive(Copy, Clone) struct CloneFromFn(CloneFromFnType));
impl_fn_wrapper!(derive(Copy, Clone) struct DropFn(DropFnType));
impl_fn_wrapper!(derive(Copy, Clone) struct EqFn(EqFnType));
impl_fn_wrapper!(derive(Copy, Clone) struct HashFn(HashFnType));
impl_fn_wrapper!(derive(Copy, Clone) struct FmtFn(FmtFnType));