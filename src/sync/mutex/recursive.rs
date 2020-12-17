use crate::sync::mutex::GenericMutex;
use core::ops::{DerefMut, Deref};

pub(crate) type EncRecursiveUnit = u64;

#[derive(Copy, Clone, Debug)]
pub(crate) struct RecursiveUnit {
    /// max u2
    pub core: u64,

    /// max u16
    pub count: u64,
    pub recursion: u64, // max u16
}

impl RecursiveUnit {
    pub const EMPTY: RecursiveUnit = RecursiveUnit { core: 0, count: 0, recursion: 0 };
}

impl From<EncRecursiveUnit> for RecursiveUnit {
    fn from(unit: EncRecursiveUnit) -> Self {
        RecursiveUnit {
            core: unit & 0b11,
            count: (unit >> 2) & 0xFF_FF,
            recursion: (unit >> 18) & 0xFF_FF,
        }
    }
}

impl Into<EncRecursiveUnit> for RecursiveUnit {
    fn into(self) -> EncRecursiveUnit {
        self.core | (self.count << 2) | (self.recursion << 18)
    }
}

pub trait RecursiveMutex: GenericMutex {
    type Token;

    unsafe fn increment_recursion_unchecked(&self) -> Self::Token;

    unsafe fn decrement_recursion_unchecked(&self, token: Self::Token);

}

pub struct RecursiveGuard<'a, M: RecursiveMutex + 'a> {
    lock: &'a M,
}

impl<'a, M: RecursiveMutex + 'a> RecursiveGuard<'a, M> {
    /// unsafe because the caller must guarantee that the eventual call to unlock_unchecked()
    /// is safe.
    pub unsafe fn new(lock: &'a M) -> Self {
        Self { lock }
    }

    pub fn recursion<R, F: FnOnce() -> R>(&mut self, func: F) -> R {
        let token = unsafe { self.lock.increment_recursion_unchecked() };

        let result = func();

        unsafe { self.lock.decrement_recursion_unchecked(token) };

        result
    }
}

impl<'a, M: RecursiveMutex> ! Send for RecursiveGuard<'a, M> {}

unsafe impl<'a, M: RecursiveMutex + 'a> Sync for RecursiveGuard<'a, M> where M::Target: Sync {}

impl<'a, M: RecursiveMutex + 'a> Deref for RecursiveGuard<'a, M> {
    type Target = M::Target;

    fn deref(&self) -> &M::Target {
        unsafe { &*self.lock.get_unchecked() }
    }
}

impl<'a, M: RecursiveMutex + 'a> DerefMut for RecursiveGuard<'a, M> {
    fn deref_mut(&mut self) -> &mut M::Target {
        unsafe { &mut *self.lock.get_unchecked() }
    }
}

impl<'a, M: RecursiveMutex + 'a> Drop for RecursiveGuard<'a, M> {
    fn drop(&mut self) {
        unsafe { self.lock.unlock_unchecked() };
    }
}

