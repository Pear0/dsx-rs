use core::cell::UnsafeCell;
use core::marker::PhantomData;

use crate::sync::{AtomicBool, AtomicU64, Ordering};
use crate::sync::mutex::{GenericMutex, LockableMutex, MutexGuard};
use crate::sync::mutex::recursive::RecursiveUnit;

pub trait BootInfo {
    fn core() -> usize;
    fn can_use_cas() -> bool;

    fn can_lock_without_cas() -> bool {
        Self::core() == 0
    }
}

pub struct BootMutex<T, B> {
    lock_unit: AtomicU64,
    value: UnsafeCell<T>,
    _phantom: PhantomData<B>,
}

unsafe impl<T, B: BootInfo> Send for BootMutex<T, B> {}

unsafe impl<T, B: BootInfo> Sync for BootMutex<T, B> {}

impl<T, B: BootInfo> BootMutex<T, B> {
    pub const fn new(value: T) -> Self {
        Self {
            lock_unit: AtomicU64::new(0),
            value: UnsafeCell::new(value),
            _phantom: PhantomData,
        }
    }

    #[inline(never)]
    fn try_early_boot_lock(&self) -> Option<MutexGuard<'_, Self>> {
        let core = B::core();
        if !B::can_lock_without_cas() {
            return None;
        }

        if self.lock_unit.load(Ordering::SeqCst) != 0 {
            return None;
        }

        self.lock_unit.store(RecursiveUnit { recursion: 0, core: core as u64, count: 1 }.into(), Ordering::SeqCst);
        Some(MutexGuard { lock: &self })
    }

}

impl<T, B: BootInfo> LockableMutex for BootMutex<T, B> {
    fn try_lock(&self) -> Option<MutexGuard<'_, Self>> {
        if unsafe { core::intrinsics::unlikely(!B::can_use_cas()) } {
            return self.try_early_boot_lock();
        }

        let this = B::core();
        let current_unit = self.lock_unit.load(Ordering::Relaxed);
        let mut unit: RecursiveUnit = current_unit.into();

        // somebody is locking this lock and hasn't performed an unlock.
        if unit.count != unit.recursion {
            return None;
        }

        // recursive locking is not allowed across cores. but if count == 0, cores
        // doesn't matter (first lock).
        if unit.count > 0 && unit.core as usize != this {
            return None;
        }

        unit.core = this as u64;
        unit.count += 1;

        // can we acquire lock
        if self.lock_unit.compare_and_swap(current_unit, unit.into(), Ordering::Acquire) != current_unit {
            return None;
        }

        Some(MutexGuard { lock: &self })
    }
}

impl<T, B: BootInfo> GenericMutex for BootMutex<T, B> {
    type Target = T;

    unsafe fn get_unchecked(&self) -> *mut Self::Target {
        self.value.get()
    }

    unsafe fn unlock_unchecked(&self) {
        self.lock_unit.store(RecursiveUnit::EMPTY.into(), Ordering::Release);
    }
}




