use crate::alloc::sync::Arc;
use crate::alloc::vec::Vec;
use crate::core::cell::UnsafeCell;
use crate::core::cmp::min;
use crate::core::fmt;
use crate::core::mem::MaybeUninit;
use crate::core::sync::atomic::Ordering;
use crate::sync::AtomicUsize;

/// [read_ptr, write_ptr) available for reading by consumer
/// [write_ptr, read_ptr) available for writing by producer
/// for simplicity read_ptr == write_ptr always means empty.
pub struct SpscQueue<T> {
    data: Vec<UnsafeCell<MaybeUninit<T>>>,
    write_ptr: AtomicUsize,
    read_ptr: AtomicUsize,
}

#[derive(Debug)]
pub struct SpscQueueReader<T>(Arc<SpscQueue<T>>);

#[derive(Debug)]
pub struct SpscQueueWriter<T>(Arc<SpscQueue<T>>);

impl<T: Send> SpscQueue<T> {
    pub fn new(size: usize) -> (SpscQueueReader<T>, SpscQueueWriter<T>) {
        let mut data = Vec::new();
        data.reserve_exact(size);
        data.resize_with(size, || UnsafeCell::new(MaybeUninit::uninit()));

        let arc = Arc::new(Self {
            data,
            write_ptr: AtomicUsize::new(0),
            read_ptr: AtomicUsize::new(0),
        });

        (SpscQueueReader(arc.clone()), SpscQueueWriter(arc))
    }
}

impl<T> fmt::Debug for SpscQueue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(core::any::type_name::<Self>())
            .field("len", &self.load_len())
            .field("cap", &self.cap())
            .finish()
    }
}

impl<T> SpscQueue<T> {
    unsafe fn take_unchecked(&self, idx: usize) -> T {
        core::mem::replace(&mut *self.data[idx].get(), MaybeUninit::uninit()).assume_init()
    }

    unsafe fn place_unchecked(&self, idx: usize, value: T) {
        let _ = core::mem::replace(&mut *self.data[idx].get(), MaybeUninit::new(value));
    }

    fn buffer_len(&self) -> usize {
        self.data.len()
    }

    /// the buffer cannot be completely filled so the
    /// available capacity is one less than the internal
    /// buffer length.
    fn cap(&self) -> usize {
        self.buffer_len() - 1
    }

    fn inc(&self, idx: usize) -> usize {
        (idx + 1) % self.buffer_len()
    }

    fn calc_len(&self, read: usize, write: usize) -> usize {
        if write >= read {
            write - read
        } else {
            self.buffer_len() + write - read
        }
    }

    fn calc_free_space(&self, read: usize, write: usize) -> usize {
        self.cap() - self.calc_len(read, write)
    }

    fn load_len(&self) -> usize {
        let read = self.read_ptr.load(Ordering::Acquire);
        let write = self.write_ptr.load(Ordering::Acquire);
        self.calc_len(read, write)
    }
}

impl<T> Drop for SpscQueue<T> {
    fn drop(&mut self) {
        let mut idx = self.read_ptr.load(Ordering::Acquire);
        let end = self.write_ptr.load(Ordering::Acquire);
        while idx != end {
            drop(unsafe { self.take_unchecked(idx) });
            idx = self.inc(idx);
        }
    }
}

impl<T: Send> SpscQueueReader<T> {
    pub fn try_dequeue(&mut self) -> Option<T> {
        let read_idx = self.0.read_ptr.load(Ordering::Acquire);
        let write_idx = self.0.write_ptr.load(Ordering::Acquire);
        if read_idx == write_idx {
            return None;
        }

        let value = unsafe { self.0.take_unchecked(read_idx) };

        self.0.read_ptr.store(self.0.inc(read_idx), Ordering::Release);

        Some(value)
    }

    pub fn dequeue_slice(&mut self, slice: &mut [T]) -> usize {
        let write_idx = self.0.write_ptr.load(Ordering::Acquire);
        let read_idx = self.0.read_ptr.load(Ordering::Acquire);

        let amt = min(slice.len(), self.0.calc_len(read_idx, write_idx));
        let mut idx = read_idx;

        for i in 0..amt {
            slice[i] = unsafe { self.0.take_unchecked(idx) };
            idx = self.0.inc(idx);
        }

        if amt > 0 {
            self.0.read_ptr.store(idx, Ordering::Release);
        }

        amt
    }

    pub fn cap(&self) -> usize {
        self.0.cap()
    }

    pub fn len(&self) -> usize {
        self.0.load_len()
    }
}

impl<T: Send> SpscQueueWriter<T> {
    pub fn try_enqueue(&mut self, value: T) -> Result<(), T> {
        let write_idx = self.0.write_ptr.load(Ordering::Acquire);
        let read_idx = self.0.read_ptr.load(Ordering::Acquire);

        // this is considered full for signaling simplicity.
        if self.0.inc(write_idx) == read_idx {
            return Err(value);
        }

        unsafe { self.0.place_unchecked(write_idx, value) };

        self.0.write_ptr.store(self.0.inc(write_idx), Ordering::Release);

        Ok(())
    }

    pub fn cap(&self) -> usize {
        self.0.cap()
    }

    pub fn len(&self) -> usize {
        self.0.load_len()
    }
}

impl<T: Clone + Send> SpscQueueWriter<T> {
    pub fn enqueue_slice(&mut self, slice: &[T]) -> usize {
        let write_idx = self.0.write_ptr.load(Ordering::Acquire);
        let read_idx = self.0.read_ptr.load(Ordering::Acquire);

        let amt = min(slice.len(), self.0.calc_free_space(read_idx, write_idx));
        let mut idx = write_idx;

        for i in 0..amt {
            unsafe { self.0.place_unchecked(idx, slice[i].clone()) };
            idx = self.0.inc(idx);
        }

        if amt > 0 {
            self.0.write_ptr.store(idx, Ordering::Release);
        }

        amt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Eq, PartialEq)]
    struct Item(pub usize);

    impl Item {
        #[track_caller]
        pub fn into_assert(self, num: usize) {
            assert_eq!(self.0, num);
            core::mem::forget(self);
        }

        #[track_caller]
        pub fn opt_assert(this: Option<Self>, num: Option<usize>) {
            assert_eq!(this.as_ref().map(|x| x.0), num);
            core::mem::forget(this);
        }
    }

    impl Drop for Item {
        fn drop(&mut self) {
            use std::thread::panicking;
            if !panicking() {
                panic!("Unexpected drop of {:?}", self);
            }
        }
    }

    #[test]
    fn construct_queue() {
        let (_read, _write) = SpscQueue::<u8>::new(1024);
    }

    #[test]
    fn basic_ops() {
        let (mut read, mut write) = SpscQueue::<Item>::new(4);

        assert_eq!(read.cap(), 3);

        Item::opt_assert(write.try_enqueue(Item(1)).err(), None);
        assert_eq!(read.len(), 1);
        Item::opt_assert(write.try_enqueue(Item(2)).err(), None);
        Item::opt_assert(write.try_enqueue(Item(3)).err(), None);

        // queue is now full.
        Item::opt_assert(write.try_enqueue(Item(4)).err(), Some(4));
        assert_eq!(read.len(), 3);

        Item::opt_assert(read.try_dequeue(), Some(1));

        Item::opt_assert(write.try_enqueue(Item(5)).err(), None);

        Item::opt_assert(write.try_enqueue(Item(6)).err(), Some(6));
        assert_eq!(read.len(), 3);

        Item::opt_assert(read.try_dequeue(), Some(2));

        Item::opt_assert(write.try_enqueue(Item(7)).err(), None);

        Item::opt_assert(write.try_enqueue(Item(8)).err(), Some(8));
        assert_eq!(read.len(), 3);

        Item::opt_assert(read.try_dequeue(), Some(3));
        Item::opt_assert(read.try_dequeue(), Some(5));
        Item::opt_assert(read.try_dequeue(), Some(7));
        assert_eq!(read.len(), 0);

        Item::opt_assert(read.try_dequeue(), None);
    }

    #[test]
    #[should_panic]
    fn items_dropped() {
        let (_read, mut write) = SpscQueue::<Item>::new(4);
        Item::opt_assert(write.try_enqueue(Item(1)).err(), None);
    }
}
