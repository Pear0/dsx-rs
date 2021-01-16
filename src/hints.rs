#![allow(unused_unsafe)]

#[must_use]
#[inline(always)]
pub fn likely(b: bool) -> bool {
    unsafe { core::intrinsics::likely(b) }
}

#[must_use]
#[inline(always)]
pub fn unlikely(b: bool) -> bool {
    unsafe { core::intrinsics::unlikely(b) }
}
