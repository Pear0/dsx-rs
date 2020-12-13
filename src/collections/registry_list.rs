use crate::sync::{AtomicBool, AtomicPtr, AtomicRef, AtomicUsize, Ordering, RwSemaphore, spin_loop_hint, RwWriteGuard};

#[derive(Debug)]
pub struct IntrusiveInfo<T: IntrusiveNode> {
    left_lock: RwSemaphore,
    center_lock: RwSemaphore,

    /// true if this node is currently in a list. A lone node in a list would
    /// otherwise be indistinguishable from an unlinked node.
    in_list: AtomicBool,

    /// pointer to next element in linked list or null if tail
    next_element: AtomicRef<T>,

    /// pointer to prev element in linked list or null if head
    prev_element: AtomicRef<T>,
}

impl<T: IntrusiveNode> IntrusiveInfo<T> {
    pub const fn new() -> Self {
        Self {
            left_lock: RwSemaphore::new(),
            center_lock: RwSemaphore::new(),
            in_list: AtomicBool::new(false),
            next_element: AtomicRef::new_null(),
            prev_element: AtomicRef::new_null(),
        }
    }

    pub fn in_list(&self) -> bool {
        self.in_list.load(Ordering::Relaxed)
    }
}

impl<T: IntrusiveNode> Default for IntrusiveInfo<T> {
    fn default() -> Self {
        Self {
            left_lock: RwSemaphore::new(),
            center_lock: RwSemaphore::new(),
            in_list: AtomicBool::new(false),
            next_element: AtomicRef::new_null(),
            prev_element: AtomicRef::new_null(),
        }
    }
}

pub trait IntrusiveNode: Sync {
    fn get_info(&self) -> &IntrusiveInfo<Self> where Self: Sized;
}

#[inline(always)]
fn retry_loop<F, R>(mut func: F) -> R
    where F: FnMut() -> Option<R> {
    loop {
        match func() {
            None => {
                spin_loop_hint();
                continue;
            }
            Some(value) => return value,
        }
    }
}

pub struct RegistryList<T> {
    head: AtomicRef<T>,
    lock: RwSemaphore,
    size: AtomicUsize,
    op_count: AtomicUsize,
}

impl<T> RegistryList<T> {
    #[cfg(feature = "loom_tests")]
    pub fn new() -> Self {
        Self {
            head: AtomicRef::new_null(),
            lock: RwSemaphore::new(),
            size: AtomicUsize::new(0),
            op_count: AtomicUsize::new(0),
        }
    }

    #[cfg(not(feature = "loom_tests"))]
    pub const fn new() -> Self {
        Self::new_const()
    }

    #[cfg(not(feature = "loom_tests"))]
    pub const fn new_const() -> Self {
        Self {
            head: AtomicRef::new_null(),
            lock: RwSemaphore::new(),
            size: AtomicUsize::new(0),
            op_count: AtomicUsize::new(0),
        }
    }
}

impl<T: IntrusiveNode> RegistryList<T> {
    pub fn size(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    pub fn op_count(&self) -> usize {
        self.op_count.load(Ordering::Relaxed)
    }

    // the caller MUST call remove before the node is destructed
    // this could be done in drop().
    pub unsafe fn insert(&self, node: &T) {
        let info = node.get_info();
        self.op_count.fetch_add(1, Ordering::Relaxed);

        assert!(!info.in_list.load(Ordering::Relaxed));

        // TODO change these to asserts
        info.prev_element.store(None, Ordering::Relaxed);
        info.next_element.store(None, Ordering::Relaxed);

        let _root_guard = self.lock.write();

        let head = match self.head.load(Ordering::Relaxed) {
            None => {
                // simple case empty list.

                self.head.store(Some(node), Ordering::Relaxed);
                self.size.fetch_add(1, Ordering::Relaxed);

                info.in_list.store(true, Ordering::Relaxed);

                return;
            }
            Some(head) => head,
        };

        let head_info = head.get_info();
        let _head_left_guard = head_info.left_lock.write();

        // link nodes.
        head_info.prev_element.store(Some(node), Ordering::Relaxed);
        info.next_element.store(Some(head), Ordering::Relaxed);

        // move list head ptr
        self.head.store(Some(node), Ordering::Relaxed);

        self.size.fetch_add(1, Ordering::Relaxed);

        info.in_list.store(true, Ordering::Relaxed);

        drop(_head_left_guard);
        drop(_root_guard);
    }

    pub fn remove(&self, node: &T) {
        let info = node.get_info();
        self.op_count.fetch_add(1, Ordering::Relaxed);
        assert!(info.in_list.load(Ordering::Relaxed));

        retry_loop(|| {
            let _left_read_guard = info.left_lock.read();

            let prev = unsafe { info.prev_element.load(Ordering::Relaxed) };
            let _prev_guard = match prev {
                None => self.lock.reserve_try_acquire_write()?,
                Some(prev) => prev.get_info().center_lock.reserve_try_acquire_write()?,
            };

            let _write_guard = info.center_lock.reserve_try_acquire_write()?;

            let _left_write_guard = _left_read_guard.try_upgrade().ok()
                .expect("failed to upgrade read, when we should be exclusive reader");

            // assert_eq!(prev.map(|x| x as *const _), unsafe { info.prev_element.load(Ordering::Relaxed) }.map(|x| x as *const _));

            let next = unsafe { info.next_element.load(Ordering::Relaxed) };
            let _next_guard = match next {
                None => None,
                Some(next) => Some(next.get_info().left_lock.reserve_try_acquire_write()?),
            };

            // at this point, we hold:
            // - center write guard on the previous node (or the list head if we are first)
            // - our left write guard
            // - our center write guard
            // - left write guard on next node.

            match prev {
                Some(prev) => prev.get_info().next_element.store(next, Ordering::Relaxed),
                None => self.head.store(next, Ordering::Relaxed),
            }

            if let Some(next) = next {
                next.get_info().prev_element.store(prev, Ordering::Relaxed);
            }

            info.prev_element.store(None, Ordering::Relaxed);
            info.next_element.store(None, Ordering::Relaxed);
            info.in_list.store(false, Ordering::Relaxed);

            self.size.fetch_sub(1, Ordering::Relaxed);

            drop(_next_guard);
            drop(_left_write_guard);
            drop(_write_guard);
            drop(_prev_guard);

            Some(())
        });

    }

    pub fn iter_ref(&self) -> impl Iterator<Item=&T> {
        ConcurrentListIterator::new(self)
    }

}

struct ConcurrentListIterator<'a, T: IntrusiveNode> {
    list: &'a RegistryList<T>,
    start: bool,
    node: Option<&'a T>,
}

impl<'a, T: IntrusiveNode> ConcurrentListIterator<'a, T> {
    fn new(list: &'a RegistryList<T>) -> Self {
        Self { list, start: true, node: None }
    }
}

impl<'a, T: IntrusiveNode> Iterator for ConcurrentListIterator<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        // called next on completed iterator
        if self.node.is_none() && !self.start {
            return None;
        }

        if self.start {
            assert!(self.node.is_none());
            self.start = false;

            let _root_guard = self.list.lock.read();

            let head = unsafe { self.list.head.load(Ordering::Relaxed)? };

            // acquire read lock on node. we will release when next() is called again.
            unsafe { head.get_info().center_lock.read().forget() };

            self.node = Some(head);

            drop(_root_guard);

            return self.node;
        }

        let node = self.node.unwrap();

        // we hold read lock from the last call to next().
        let next = unsafe { node.get_info().next_element.load(Ordering::Relaxed) };

        match next {
            None => {
                // we are done. release lock and exit.
                unsafe { node.get_info().center_lock.release_read() };
                self.node = None;
                self.node
            }
            Some(next) => {
                // first acquire lock on next node which we will keep until call to next again.
                unsafe { next.get_info().center_lock.read().forget() };

                self.node = Some(next);

                // then release lock on old/current node
                unsafe { node.get_info().center_lock.release_read() };

                self.node
            }
        }
    }
}

impl<'a, T: IntrusiveNode> Drop for ConcurrentListIterator<'a, T> {
    fn drop(&mut self) {
        if let Some(node) = self.node {
            unsafe { node.get_info().center_lock.release_read() };
        }
    }
}


#[cfg(test)]
mod tests {
    use crate::alloc::boxed::Box;

    #[cfg(feature = "loom_tests")]
    use loom::sync::Arc;
    #[cfg(not(feature = "loom_tests"))]
    use crate::alloc::sync::Arc;

    #[cfg(feature = "loom_tests")]
    use loom::sync::Mutex;
    #[cfg(not(feature = "loom_tests"))]
    use crate::core::sync::Mutex;

    #[cfg(feature = "loom_tests")]
    use loom::thread;
    #[cfg(not(feature = "loom_tests"))]
    use std::thread;

    #[cfg(feature = "loom_tests")]
    const LOOM: bool = true;
    #[cfg(not(feature = "loom_tests"))]
    const LOOM: bool = false;

    use super::*;
    use test::Bencher;
    use std::time::Duration;

    struct SimpleNode {
        value: usize,
        info: IntrusiveInfo<Self>,
    }

    impl SimpleNode {
        pub fn new(value: usize) -> Self {
            Self { value, info: IntrusiveInfo::default() }
        }
    }

    impl IntrusiveNode for SimpleNode {
        fn get_info(&self) -> &IntrusiveInfo<Self> where Self: Sized {
            &self.info
        }
    }

    struct Scope<'a> {
        threads: Mutex<Vec<thread::JoinHandle<()>>>,
        __phantom: core::marker::PhantomData<&'a ()>,
    }

    impl<'a> Scope<'a> {
        fn spawn<F>(&self, f: F) where
            F: FnOnce() + Send + 'a {

            let closure: Box<dyn FnOnce() + Send + 'a> = Box::new(f);
            let closure: Box<dyn FnOnce() + Send + 'static> =
                unsafe { core::mem::transmute(closure) };

            let handle = thread::spawn(move || {
                closure();
            });

            let mut lock = self.threads.lock().unwrap();
            lock.push(handle);

        }
    }

    fn scope<'a, F, R>(f: F) -> R where
        F: FnOnce(&Scope<'a>) -> R {

        let mut scope = Scope { threads: Mutex::new(Vec::new()), __phantom: core::marker::PhantomData };
        let result = f(&scope);

        let mut lock = scope.threads.lock().unwrap();
        for t in lock.drain(..) {
            t.join().unwrap();
        }

        result
    }

    fn loom_wrapper<F>(f: F)
        where F: Fn() + Sync + Send + 'static {
        #[cfg(feature = "loom_tests")]
            {
                loom::model(f);
            }
        #[cfg(not(feature = "loom_tests"))]
            {
                f();
            }
    }

    fn leak<T>(t: T) -> &'static T {
        Box::leak(Box::new(t))
    }

    #[test]
    #[cfg_attr(feature = "loom_tests", ignore)]
    fn no_loom_test() {}

    #[test]
    fn insert_one() {
        loom_wrapper(|| {
            let list = RegistryList::new();
            let node = leak(SimpleNode::new(0));
            unsafe {
                list.insert(node);
            }

            assert_eq!(list.size(), 1);

            let c = list.iter_ref().count();
            assert_eq!(c, 1);
        });
    }

    #[test]
    fn insert_remove() {
        loom_wrapper(|| {
            let list = RegistryList::new();
            let node = leak(SimpleNode::new(0));
            unsafe {
                list.insert(node);
            }

            assert_eq!(list.size(), 1);
            let c = list.iter_ref().count();
            assert_eq!(c, 1);

            list.remove(node);

            assert_eq!(list.size(), 0);
            let c = list.iter_ref().count();
            assert_eq!(c, 0);

        });
    }

    #[test]
    fn insert_while_removing() {
        const NUM_THREADS: usize = if LOOM { 2 } else { 6 };
        const NUM_INSERTS: usize = if LOOM { 4 } else { 100000 };

        loom_wrapper(|| {
            let list: Arc<RegistryList<SimpleNode>> = Arc::new(RegistryList::new());
            let active_count = AtomicUsize::new(0);

            scope(|s| {
                for i in 0..NUM_THREADS {
                    active_count.fetch_add(1, Ordering::Relaxed);
                    // let list = list.clone();
                    s.spawn(|| {
                        let node = leak(SimpleNode::new(0));

                        for j in 0..NUM_INSERTS {
                            unsafe {
                                list.insert(node);
                            }

                            list.remove(node);
                        }

                        active_count.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            });

            assert_eq!(list.size(), 0);
            let c = list.iter_ref().count();
            assert_eq!(c, 0);

        });
    }

    #[test]
    fn insert_many() {
        const NUM_THREADS: usize = if LOOM { 2 } else { 4 };
        const NUM_INSERTS: usize = if LOOM { 4 } else { 10000 };

        loom_wrapper(|| {
            let list = Arc::new(RegistryList::new());

            scope(|s| {
                for i in 0..NUM_THREADS {
                    let list = list.clone();
                    s.spawn(move || {
                        for j in 0..NUM_INSERTS {
                            let node = leak(SimpleNode::new(0));
                            unsafe {
                                list.insert(node);
                            }
                        }
                    });
                }
            });

            assert_eq!(list.size(), NUM_THREADS * NUM_INSERTS);

            let c = list.iter_ref().count();
            assert_eq!(c, NUM_THREADS * NUM_INSERTS);
        });
    }

    #[test]
    #[cfg_attr(feature = "loom_tests", ignore)]
    fn insert_many_while_iterating() {
        const NUM_THREADS: usize = 6;
        const NUM_ITER_THREADS: usize = 2;
        const NUM_INSERTS: usize = 10000;

        loom_wrapper(|| {
            let list = Arc::new(RegistryList::new());

            scope(|s| {
                for i in 0..NUM_ITER_THREADS {
                    let list = list.clone();
                    s.spawn(move || {
                        while list.size() < NUM_THREADS * NUM_INSERTS {
                            list.iter_ref().count();
                        }
                    });
                }

                for i in 0..NUM_THREADS {
                    let list = list.clone();
                    s.spawn(move || {
                        for j in 0..NUM_INSERTS {
                            let node = leak(SimpleNode::new(0));
                            unsafe {
                                list.insert(node);
                            }
                        }
                    });
                }
            });

            assert_eq!(list.size(), NUM_THREADS * NUM_INSERTS);

            let c = list.iter_ref().count();
            assert_eq!(c, NUM_THREADS * NUM_INSERTS);
        });
    }

    #[bench]
    fn bench_add_two(b: &mut Bencher) {
        let list: RegistryList<SimpleNode> = RegistryList::new();

        let node1 = leak(SimpleNode::new(0));
        unsafe {
            list.insert(node1);
        }

        let exit = AtomicBool::new(false);

        scope(|s| {

            for _ in 0..3 {
                s.spawn(|| {
                    let node = leak(SimpleNode::new(0));
                    while !exit.load(Ordering::Relaxed) {
                        unsafe {
                            list.insert(node);
                            list.remove(node);
                        }
                    }
                });
            }

            let node = leak(SimpleNode::new(0));
            b.iter(|| {
                unsafe {
                    list.insert(node);
                    list.remove(node);
                }
            });
            exit.store(true, Ordering::Relaxed);
        });


    }

}




