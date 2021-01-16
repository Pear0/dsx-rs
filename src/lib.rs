#![feature(const_caller_location)]
#![feature(const_fn)]
#![feature(core_intrinsics)]
#![feature(negative_impls)]
#![feature(test)]

#![cfg_attr(not(feature = "std"), no_std)]

// #![cfg_attr(feature = "kern", feature(const_fn))]

#[cfg(feature = "std")]
pub use std as core;
#[cfg(feature = "std")]
pub use std as alloc;
#[cfg(not(feature = "std"))]
pub use core;

#[cfg(not(feature = "std"))]
pub extern crate alloc;

pub mod collections;

pub mod hints;

#[cfg(feature = "kern")]
pub mod kern;

pub mod sync;

pub mod sys;

#[macro_use]
extern crate downcast_rs;

#[cfg(test)]
extern crate test;
