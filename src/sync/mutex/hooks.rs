use crate::core::cell::UnsafeCell;
use crate::core::panic::Location;
use crate::core::time::Duration;
use crate::sync::mutex::{GenericMutex, LockableMutex, MutexDataContainerSync, RecursiveMutex};

pub struct LockContext {
    pub time_to_lock: Duration,
    pub lock_name: &'static Location<'static>,
    pub locked_by: &'static Location<'static>,
}

pub trait LockHooks: Send + Sync + 'static {
    fn on_locked(&self, item: &mut MutexDataContainerSync, ctx: &LockContext);

    fn lock_dropped(&self, _item: MutexDataContainerSync) {}
}

struct NopLockHooks;

impl LockHooks for NopLockHooks {
    fn on_locked(&self, item: &mut MutexDataContainerSync, ctx: &LockContext) {}
}

pub static mut HOOKS: &dyn LockHooks = &NopLockHooks;

#[derive(Debug)]
pub struct HookedMutex<T> {
    lock: T,
    data: UnsafeCell<MutexDataContainerSync>,
    lock_name: &'static Location<'static>,
}

impl<T> HookedMutex<T> {
    #[track_caller]
    pub const fn new(lock: T) -> Self {
        Self {
            lock,
            data: UnsafeCell::new(MutexDataContainerSync::empty()),
            lock_name: Location::caller(),
        }
    }
}

unsafe impl<T> Sync for HookedMutex<T> {}

impl<'a, T: LockableMutex<'a>> LockableMutex<'a> for HookedMutex<T> {
    type Guard = T::Guard;

    #[track_caller]
    fn try_lock_info(&'a self, trying_since: Option<&Duration>) -> Option<Self::Guard> {
        let guard = self.lock.try_lock_info(trying_since)?;

        let ctx = LockContext {
            time_to_lock: Duration::default(),
            lock_name: self.lock_name,
            locked_by: Location::caller(),
        };

        unsafe { HOOKS.on_locked(&mut *self.data.get(), &ctx) };

        Some(guard)
    }
}

impl<T: GenericMutex> GenericMutex for HookedMutex<T> {
    type Target = T::Target;

    unsafe fn get_unchecked(&self) -> *mut Self::Target {
        self.lock.get_unchecked()
    }

    unsafe fn unlock_unchecked(&self) {
        self.lock.unlock_unchecked()
    }
}

impl<T: RecursiveMutex> RecursiveMutex for HookedMutex<T> {
    type Token = T::Token;

    unsafe fn increment_recursion_unchecked(&self) -> Self::Token {
        self.lock.increment_recursion_unchecked()
    }

    unsafe fn decrement_recursion_unchecked(&self, token: Self::Token) {
        self.lock.decrement_recursion_unchecked(token)
    }
}

impl<T> Drop for HookedMutex<T> {
    fn drop(&mut self) {
        unsafe {
            let data = core::mem::replace(&mut *self.data.get(), MutexDataContainerSync::empty());
            if data.has_data() {
                HOOKS.lock_dropped(data);
            }
        }
    }
}