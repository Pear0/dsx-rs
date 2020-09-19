
#![cfg_attr(not(feature = "std"), no_std)]

#![cfg_attr(feature = "kern", feature(const_fn))]

#[cfg(feature = "std")]
pub(crate) use std as core;
#[cfg(feature = "std")]
pub(crate) use std as alloc;

#[cfg(not(feature = "std"))]
pub(crate) use core;

#[cfg(not(feature = "std"))]
pub(crate) extern crate alloc;

#[cfg(feature = "kern")]
pub mod kern;

pub mod sync;
