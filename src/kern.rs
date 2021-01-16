use crate::core::marker::PhantomData;
use crate::core::cell::UnsafeCell;
use crate::core::sync::atomic::{AtomicBool, Ordering};
use crate::core::mem::ManuallyDrop;

/// Implement the kernel hooks for InterruptRcu.
///
/// While Rcu is safe in theory, incorrectly implementing these hooks can make it unsafe.
pub trait KernInterruptRcu: Send + Sync {

    /// Execute a function with interrupts disabled. This function does not need to be re-entrant.
    fn critical_region<R, F: FnOnce() -> R>(func: F) -> R;

    /// Get a total irq count for a specific core. Exactly which irqs are counted does not
    /// matter but this count must not increase for a given core while that core is
    /// executing critical_region().
    fn get_core_irq_count(core: usize) -> u32;

    fn core_count() -> usize;

    /// Index of the current core.
    fn my_core_index() -> usize;

    /// Sleep for some amount of time before checking irq counts again.
    fn yield_for_other_cores();

    /// Ensure a value is visible to other cores.
    fn memory_barrier<T>(value: &T);

    /// Hint that a spin loop is occurring.
    fn lock_failed() {
    }
}



pub struct InterruptRcu<T, K: KernInterruptRcu> {
    cell: UnsafeCell<ManuallyDrop<T>>,
    update_lock: AtomicBool,
    __phantom: PhantomData<K>,
}

unsafe impl<T, K: KernInterruptRcu> Sync for InterruptRcu<T, K> {}

impl<T, K: KernInterruptRcu> InterruptRcu<T, K> {
    pub const MAX_CORES: usize = 32;

    pub const fn new(value: T) -> Self {
        Self {
            cell: UnsafeCell::new(ManuallyDrop::new(value)),
            update_lock: AtomicBool::new(false),
            __phantom: PhantomData,
        }
    }

    pub fn critical<R, F: FnOnce(&T) -> R>(&self, func: F) -> R {
        K::critical_region(|| {
            let r = unsafe { &*self.cell.get() };
            func(r)
        })
    }

    pub fn update<F: FnOnce(&T) -> T>(&self, func: F) {
        // Acquire the Update lock
        while self.update_lock.compare_and_swap(false, true, Ordering::Acquire) != false {
            K::lock_failed();
        }

        // ==== Read + Copy ====
        let new_copy = {
            let r = K::critical_region(|| unsafe { &*self.cell.get() } );
            func(r)
        };

        let mut old_copy = K::critical_region(|| {
            // ==== Update ====
            let old_copy = core::mem::replace(unsafe { &mut *self.cell.get() }, ManuallyDrop::new(new_copy));
            K::memory_barrier(unsafe { &*self.cell.get() });
            old_copy
        });

        self.wait_for_all_critical_zones();

        // Everyone else is done, drop the old value...
        unsafe { ManuallyDrop::drop(&mut old_copy); core::mem::drop(old_copy) };

        // release the lock
        self.update_lock.store(false, Ordering::Release);

    }

    fn wait_for_all_critical_zones(&self) {
        let mut initial_counts = [0u32; Self::MAX_CORES];
        let core_count = K::core_count();
        for i in 0..core_count {
            initial_counts[i] = K::get_core_irq_count(i);
        }

        let mut had_irq = [false; Self::MAX_CORES];
        had_irq[K::my_core_index()] = true;

        loop {
            for i in 0..core_count {
                if had_irq[i] {
                    continue;
                }

                if K::get_core_irq_count(i) > initial_counts[i] {
                    had_irq[i] = true;
                }
            }

            if had_irq.iter().take(core_count).all(|x| *x) {
                return;
            } else {
                K::yield_for_other_cores()
            }
        }
    }
}




