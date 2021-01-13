use crate::core::cell::UnsafeCell;
use crate::core::intrinsics::unlikely;
use crate::core::mem::MaybeUninit;
use crate::core::ops::Deref;
use crate::core::sync::atomic::{AtomicUsize, Ordering};

const UNINIT: usize = 0;
const INITING: usize = 1;
const INITED: usize = 2;

pub struct SingleSetSemaphore<T> {
    value: UnsafeCell<MaybeUninit<T>>,
    state: AtomicUsize,
}

impl<T> SingleSetSemaphore<T> {
    pub const fn new() -> Self {
        Self {
            value: UnsafeCell::new(MaybeUninit::uninit()),
            state: AtomicUsize::new(UNINIT),
        }
    }

    pub unsafe fn set_racy(this: &Self, value: T) {
        assert_eq!(this.state.load(Ordering::Relaxed), UNINIT);
        *this.value.get() = MaybeUninit::new(value);
        this.state.store(INITED, Ordering::Release);
    }

    pub fn set(this: &Self, value: T) {
        assert_eq!(this.state.compare_and_swap(UNINIT, INITING, Ordering::Relaxed), UNINIT);
        unsafe { *this.value.get() = MaybeUninit::new(value) };
        this.state.store(INITED, Ordering::Release);
    }

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

impl<T> Deref for SingleSetSemaphore<T> {
    type Target = T;

    #[track_caller]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}
