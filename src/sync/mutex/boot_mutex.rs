use core::cell::UnsafeCell;
use core::intrinsics::unlikely;
use core::marker::PhantomData;

use crate::sync::{AtomicBool, AtomicU64, Ordering};
use crate::sync::mutex::{GenericMutex, LockableMutex, MutexGuard};
use crate::sync::mutex::recursive::{RecursiveGuard, RecursiveMutex, RecursiveUnit};

pub trait BootInfo: Sync + Send + 'static {
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
    fn try_early_boot_lock(&self) -> Option<RecursiveGuard<'_, Self>> {
        let core = B::core();
        if !B::can_lock_without_cas() {
            return None;
        }

        if self.lock_unit.load(Ordering::SeqCst) != 0 {
            return None;
        }

        self.lock_unit.store(RecursiveUnit { recursion: 0, core: core as u64, count: 1 }.into(), Ordering::SeqCst);
        Some(unsafe { RecursiveGuard::new(&self) })
    }

    #[inline(never)]
    fn early_boot_unlock(&self) {
        self.lock_unit.store(RecursiveUnit::EMPTY.into(), Ordering::Release);
    }
}

impl<'a, T: 'a, B: BootInfo> LockableMutex<'a> for BootMutex<T, B> {
    type Guard = RecursiveGuard<'a, Self>;

    fn try_lock(&'a self) -> Option<Self::Guard> {
        if unsafe { unlikely(!B::can_use_cas()) } {
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

        Some(unsafe { RecursiveGuard::new(&self) })
    }
}

impl<T, B: BootInfo> RecursiveMutex for BootMutex<T, B> {
    type Token = u64;

    unsafe fn increment_recursion_unchecked(&self) -> Self::Token {
        if unsafe { unlikely(!B::can_use_cas()) } {
            panic!("cannot use increment_recursion() before CAS is available");
        }
        let mut unit: RecursiveUnit = self.lock_unit.load(Ordering::Acquire).into();
        unit.recursion += 1;
        self.lock_unit.store(unit.into(), Ordering::Release);

        // our token is the recursion unit value we expect to see when decrementing.
        // If it doesnt match, something has gone wrong.
        unit.into()
    }

    unsafe fn decrement_recursion_unchecked(&self, token: Self::Token) {
        if unsafe { unlikely(!B::can_use_cas()) } {
            panic!("cannot use increment_recursion() before CAS is available");
        }

        let mut unit: RecursiveUnit = token.into();
        unit.recursion -= 1;

        match self.lock_unit.compare_exchange(token, unit.into(), Ordering::Release, Ordering::Relaxed) {
            Ok(_) => {}
            Err(e) => panic!("failed to decrement recursion on lock: {:#x}", e),
        }
    }
}

impl<T, B: BootInfo> GenericMutex for BootMutex<T, B> {
    type Target = T;

    unsafe fn get_unchecked(&self) -> *mut Self::Target {
        self.value.get()
    }

    unsafe fn unlock_unchecked(&self) {
        if unsafe { unlikely(!B::can_use_cas()) } {
            return self.early_boot_unlock();
        }

        let mut unit: RecursiveUnit = self.lock_unit.load(Ordering::Relaxed).into();
        unit.count -= 1;
        self.lock_unit.store(unit.into(), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestInfo;

    impl BootInfo for TestInfo {
        fn core() -> usize {
            0
        }

        fn can_use_cas() -> bool {
            true
        }
    }

    type Mutex<T> = BootMutex<T, TestInfo>;

    #[test]
    fn basic_lock_test() {
        let lock = Mutex::new(0);

        {
            let mut l = lock.try_lock().unwrap();
            *l = 5;
        }
    }

    #[test]
    fn recursion_test() {
        let lock = Mutex::new(0);

        {
            let mut l = lock.try_lock().unwrap();
            *l = 5;

            l.recursion(|| {

                let mut l2 = lock.try_lock().unwrap();
                *l2 = 6;

            });

        }
    }

    #[test]
    #[should_panic]
    fn recursion_overlap_test() {
        let lock = Mutex::new(0);

        {
            let mut l = lock.try_lock().unwrap();
            *l = 5;

            l.recursion(|| {
                let mut l2 = lock.try_lock().unwrap();
                *l2 = 6;

                // make the inner guard try to outlast the outer
                // recursive block.
                core::mem::forget(l2);
            });
        }
    }

    #[test]
    fn double_lock_test() {
        let lock = Mutex::new(0);

        let mut l = lock.try_lock().unwrap();
        assert!(lock.try_lock().is_none());
    }

}

