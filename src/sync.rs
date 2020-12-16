
pub mod mutex;

#[cfg(feature = "loom_tests")]
pub(crate) use loom::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
#[cfg(feature = "loom_tests")]
pub(crate) use loom::thread::{yield_now as spin_loop_hint};

#[cfg(not(feature = "loom_tests"))]
pub(crate) use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering, spin_loop_hint};


#[derive(Default, Debug)]
pub struct AtomicRef<T>(AtomicPtr<T>);

impl<T> AtomicRef<T> {

    #[cfg(feature = "loom_tests")]
    pub fn new_null() -> Self {
        Self(AtomicPtr::new(core::ptr::null_mut()))
    }

    #[cfg(not(feature = "loom_tests"))]
    pub const fn new_null() -> Self {
        Self(AtomicPtr::new(core::ptr::null_mut()))
    }

    pub fn store(&self, value: Option<&T>, order: Ordering) {
        let ptr = value.map(|x| x as *const _).unwrap_or(core::ptr::null());
        self.0.store(ptr as usize as *mut _, order);
    }

    pub fn load_ptr(&self, order: Ordering) -> Option<*const T> {
        let ptr = self.0.load(order);
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const _)
        }
    }

    pub unsafe fn load<'a>(&self, order: Ordering) -> Option<&'a T> {
        self.load_ptr(order).map(|x| &*x)
    }
}




#[derive(Debug, Clone, Copy)]
enum ExgOp {
    Cancel,
    Yield,
    Value(usize),
}

#[derive(Default, Debug)]
pub struct RwSemaphore(AtomicUsize);

/// 0xfff..fff - write locked (implicitly write reserve)
/// bit 0 - write reserve. if 1, readers must wait.
/// bits 63-1 - reader count
///
impl RwSemaphore {
    const MAX_READERS: usize = (usize::max_value() >> 1) - 1;

    #[cfg(feature = "loom_tests")]
    pub fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    #[cfg(not(feature = "loom_tests"))]
    pub const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    pub fn raw(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }

    fn exchange_loop<F>(&self, success: Ordering, mut func: F) -> ExgOp where F: FnMut(usize) -> ExgOp {
        let mut value = self.0.load(Ordering::Relaxed);
        loop {
            // i don't think this should be necessary but tests fail without it...
            value = self.0.load(Ordering::Relaxed);

            let new_value = match func(value) {
                ExgOp::Value(v) => v,
                ExgOp::Yield => {
                    spin_loop_hint();
                    continue;
                }
                ExgOp::Cancel => {
                    return ExgOp::Cancel;
                }
            };

            match self.0.compare_exchange(value, new_value, success, Ordering::Relaxed) {
                Ok(_) => return ExgOp::Value(new_value),
                Err(v) => {
                    value = v;
                    spin_loop_hint();
                    continue;
                },
            }
        }
    }

    pub fn read(&self) -> RwReadGuard<'_> {
        self.exchange_loop(Ordering::Acquire, |value| {
            let reserve = value & 1;
            let read_count = value >> 1;
            if reserve != 0 || read_count == Self::MAX_READERS {
                ExgOp::Yield
            } else {
                let new_value = (read_count + 1) << 1;
                ExgOp::Value(new_value)
            }
        });

        RwReadGuard(self)
    }

    pub fn write(&self) -> RwWriteGuard<'_> {
        // set reserve bit
        self.0.fetch_or(1, Ordering::Relaxed);

        self.exchange_loop(Ordering::Acquire, |value| {
            let read_count = value >> 1;
            if read_count == 0 {
                ExgOp::Value(usize::max_value())
            } else {
                ExgOp::Yield
            }
        });

        RwWriteGuard(self)
    }

    /// Set write-reserve flag and try to acquire write lock.
    /// If there are readers, will wait until a writer succeeds.
    /// Returns Some(_) if we succeeded, None if another writer succeeded.
    pub fn reserve_try_acquire_write(&self) -> Option<RwWriteGuard<'_>> {
        // set reserve bit
        self.0.fetch_or(1, Ordering::Relaxed);

        let result = self.exchange_loop(Ordering::Acquire, |value| {
            let read_count = value >> 1;
            if read_count == 0 {
                // no readers and no writers, lets try to acquire
                ExgOp::Value(usize::max_value())
            } else if value == usize::max_value() {
                // already a writer, exit
                ExgOp::Cancel
            } else {
                // readers active, yield
                ExgOp::Yield
            }
        });

        match result {
            ExgOp::Value(_) => Some(RwWriteGuard(self)),
            ExgOp::Cancel => None,
            ExgOp::Yield => unreachable!(),
        }
    }

    // caller must hold a valid read lock. If returns true, caller now owns a write lock.
    unsafe fn try_upgrade_to_write(&self) -> bool {
        let result = self.exchange_loop(Ordering::Acquire, |value| {
            let read_count = value >> 1;
            if read_count == 1 {
                // we are the only reader
                ExgOp::Value(usize::max_value())
            } else {
                ExgOp::Cancel
            }
        });

        matches!(result, ExgOp::Value(_))
    }

    pub unsafe fn release_read(&self) {
        self.exchange_loop(Ordering::Release, |value| {
            let reserve = value & 1;
            let read_count = value >> 1;
            let new_value = ((read_count - 1) << 1) | reserve;
            ExgOp::Value(new_value)
        });
    }

    pub unsafe fn release_write(&self) {
        self.0.store(0, Ordering::Release);
    }
}



pub struct RwReadGuard<'a>(&'a RwSemaphore);

impl<'a> RwReadGuard<'a> {
    pub unsafe fn forget(self) {
        core::mem::forget(self)
    }

    pub fn try_upgrade(self) -> Result<RwWriteGuard<'a>, Self> {
        if unsafe { self.0.try_upgrade_to_write() } {
            let r = Ok(RwWriteGuard(self.0));
            core::mem::forget(self);
            r
        } else {
            Err(self)
        }
    }

}

impl<'a> Drop for RwReadGuard<'a> {
    fn drop(&mut self) {
        unsafe { self.0.release_read() };
    }
}

pub struct RwWriteGuard<'a>(&'a RwSemaphore);

impl<'a> RwWriteGuard<'a> {
    pub unsafe fn forget(self) {
        core::mem::forget(self)
    }
}

impl<'a> Drop for RwWriteGuard<'a> {
    fn drop(&mut self) {
        unsafe { self.0.release_write() };
    }
}




