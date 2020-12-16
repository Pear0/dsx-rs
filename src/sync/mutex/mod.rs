mod boot_mutex;
pub(crate) mod recursive;

pub use boot_mutex::*;

use core::ops::{Deref, DerefMut};
use core::cell::UnsafeCell;
use crate::sync::{AtomicBool, Ordering};

pub trait GenericMutex: Sync + Send {
    type Target;

    unsafe fn get_unchecked(&self) -> *mut Self::Target;

    unsafe fn unlock_unchecked(&self);
}

pub trait LockableMutex: GenericMutex + Sized {

    fn try_lock(&self) -> Option<MutexGuard<'_, Self>>;

}


pub struct MutexGuard<'a, M: GenericMutex + 'a> {
    lock: &'a M,
}

impl<'a, M: GenericMutex + 'a> MutexGuard<'a, M> {
    /// unsafe because the caller must guarantee that the eventual call to unlock_unchecked()
    /// is safe.
    pub unsafe fn new(lock: &'a M) -> Self {
        Self { lock }
    }
}

impl<'a, M: GenericMutex> ! Send for MutexGuard<'a, M> {}

unsafe impl<'a, M: GenericMutex + 'a> Sync for MutexGuard<'a, M> where M::Target: Sync {}

impl<'a, M: GenericMutex + 'a> Deref for MutexGuard<'a, M> {
    type Target = M::Target;

    fn deref(&self) -> &M::Target {
        unsafe { &*self.lock.get_unchecked() }
    }
}

impl<'a, M: GenericMutex + 'a> DerefMut for MutexGuard<'a, M> {
    fn deref_mut(&mut self) -> &mut M::Target {
        unsafe { &mut *self.lock.get_unchecked() }
    }
}

impl<'a, M: GenericMutex + 'a> Drop for MutexGuard<'a, M> {
    fn drop(&mut self) {
        unsafe { self.lock.unlock_unchecked() };
    }
}

pub struct DummyMutex();

impl DummyMutex {
    pub fn lock(&self) -> MutexGuard<'_, Self> {
        unsafe { MutexGuard::new(self) }
    }
}

impl GenericMutex for DummyMutex {
    type Target = ();

    unsafe fn get_unchecked(&self) -> *mut Self::Target {
        core::ptr::null_mut()
    }

    unsafe fn unlock_unchecked(&self) {}
}


pub struct LightMutex<T>{
    lock: AtomicBool,
    value: UnsafeCell<T>,
}

impl<T> LightMutex<T> {
    pub fn new(value: T) -> Self {
        Self {
            lock: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }
}

unsafe impl<T> Send for LightMutex<T> {}

unsafe impl<T> Sync for LightMutex<T> {}

impl <T> LockableMutex for LightMutex<T> {
    fn try_lock(&self) -> Option<MutexGuard<'_, Self>> {
        match self.lock.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => Some(unsafe { MutexGuard::new(self) }),
            Err(_) => None,
        }
    }
}

impl<T> GenericMutex for LightMutex<T> {
    type Target = T;

    unsafe fn get_unchecked(&self) -> *mut Self::Target {
        self.value.get()
    }

    unsafe fn unlock_unchecked(&self) {
        self.lock.store(false, Ordering::Release);
    }
}
