use crate::core::cell::UnsafeCell;
use crate::core::intrinsics::unlikely;
use crate::core::mem::MaybeUninit;
use crate::core::ops::Deref;
use crate::core::sync::atomic::{AtomicUsize, Ordering};

const UNINIT: usize = 0;
const INITING: usize = 1;
const INITED: usize = 2;

/// A container that can be atomically initialized exactly once.
///
/// It is ideal for global static variables that are not or cannot be const
/// but where the overhead of a lock if unnecessary because the type is Sync
/// and doesn't need to be mutably referenced.
pub struct SingleSetSemaphore<T> {
    value: UnsafeCell<MaybeUninit<T>>,
    state: AtomicUsize,
}

unsafe impl<T: Sync> Sync for SingleSetSemaphore<T> {}

impl<T: Sync> SingleSetSemaphore<T> {
    /// Create an empty SingleSetSemaphore.
    pub const fn new() -> Self {
        Self {
            value: UnsafeCell::new(MaybeUninit::uninit()),
            state: AtomicUsize::new(UNINIT),
        }
    }

    /// Check if the semaphore has been initialized already.
    pub fn is_initialized(this: &Self) -> bool {
        this.state.load(Ordering::Relaxed) == INITED
    }

    /// Set the semaphore if it has not already been set in a racy way. This function
    /// is unsafe because it races with other calls to set_racy() and set().
    ///
    /// This function is available for situations when compare_and_swap() operations
    /// are not or may not be supported by the hardware. For example during the boot
    /// sequence of an Aarch64 processor, CAS will trigger an exception until virtual
    /// memory has been configured.
    pub unsafe fn set_racy(this: &Self, value: T) {
        assert_eq!(this.state.load(Ordering::Relaxed), UNINIT);
        *this.value.get() = MaybeUninit::new(value);
        this.state.store(INITED, Ordering::Release);
    }

    /// Set a value for this semaphore. Returns Err(value) with the passed in value if the semaphore
    /// has already been initialized or another thread has claimed ownership of the initialization.
    pub fn set(this: &Self, value: T) -> Result<(), T> {
        match this.state.compare_exchange(UNINIT, INITING, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => {
                unsafe { *this.value.get() = MaybeUninit::new(value) };
                this.state.store(INITED, Ordering::Release);
                Ok(())
            }
            Err(_) => {
                Err(value)
            }
        }
    }

    /// Access the internal value. Panics if the semaphore has not been initialized.
    #[track_caller]
    pub fn as_ref(&self) -> &T {
        unsafe {
            let state = self.state.load(Ordering::Acquire);
            if unlikely(state != INITED) {
                panic!("cannot use semaphore before it has been set.");
            }
            &*(&*self.value.get()).as_ptr()
        }
    }
}

impl<T: Sync> Deref for SingleSetSemaphore<T> {
    type Target = T;

    #[track_caller]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}
