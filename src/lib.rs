//! A clone of `std::sync::Once`.

use std::{
    mem,
    cell::UnsafeCell,
    thread::{self, Thread},
};

mod spinlock;

use spinlock::SpinLock;

struct Inner {
    done: bool,
    parked: Vec<Thread>,
}

impl Inner {
    fn new() -> Self {
        Self {
            done: false,
            parked: Vec::new(),
        }
    }
}

/// Like `std::sync::Once`, but relaxing the requirement of `&'static self` to
/// `&self` on `call_once`. Access to inner state is guarded by a spinlock.
///
/// ```
/// use once_nonstatic::Once;
///
/// let once = Once::INIT;
/// let mut times_called = 0;
///
/// for _ in 0..100 {
///     once.call_once(|| {
///         times_called += 1;
///     });
/// }
///
/// assert_eq!(1, times_called);
/// ```
pub struct Once {
    lock: SpinLock,
    inner: UnsafeCell<Option<Inner>>,
}

// The SpinLock guards access to the inner value. For this reason the entire
// type is Sync.
unsafe impl Sync for Once {}

impl Once {
    /// A constant initializer.
    ///
    /// The inner values of this struct are not public and as such should not be
    /// considered stable.
    pub const INIT: Once = Once {
        lock: SpinLock::INIT,
        inner: UnsafeCell::new(None),
    };

    /// Call a function only once.
    ///
    /// This behaves exactly like `std::sync::Once` with a relaxed requirement
    /// on the lifetime of `&self`.
    pub fn call_once<F: FnOnce()>(&self, f: F) {
        let mut opt = Some(f);

        unsafe {
            self.call_once_inner(&mut move || opt.take().unwrap()());
        }
    }

    unsafe fn call_once_inner(&self, f: &mut FnMut()) {
        self.lock.lock();

        let ptr = self.inner.get();

        if let Some(inner) = (*ptr).as_mut() {
            if !inner.done {
                let current = thread::current();
                inner.parked.push(current.clone());

                // It is OK if the thread unparks before we can park.
                self.lock.unlock();
                thread::park();
            } else {
                self.lock.unlock();
            }
        } else {
            *ptr = Some(Inner::new());

            // The closure may take a long time, so we unlock the data.
            self.lock.unlock();
            f();
            self.lock.lock();

            let inner = (*ptr).as_mut().unwrap();

            inner.done = true;
            let parked_threads = mem::replace(&mut inner.parked, Vec::new());

            self.lock.unlock();

            for t in parked_threads {
                t.unpark();
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        thread,
        time::Duration,
        sync::{Arc, Mutex},
    };

    use super::{SpinLock, Once};

    #[test]
    fn multi_thread_single_call() {
        static ONCE: Once = Once::INIT;

        let mut threads = Vec::new();
        let calls = Arc::new(Mutex::new(0));

        for _ in 0..10 {
            let calls = calls.clone();

            let t = thread::spawn(move || {
                // Sleeping in the call ensures all future threads park
                // themselves.
                ONCE.call_once(move || {
                    thread::sleep(Duration::from_millis(100));
                    *calls.lock().unwrap() += 1;
                });
            });

            threads.push(t);
        }

        for t in threads {
            t.join().unwrap();
        }

        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn single_thread_multi_call() {
        static ONCE: Once = Once::INIT;

        let calls = Arc::new(Mutex::new(0));

        for _ in 0..100 {
            let calls = calls.clone();

            ONCE.call_once(move || {
                *calls.lock().unwrap() += 1;
            });
        }

        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn spinlock_loop_single() {
        // If this test does not hang, it is successful.

        let lock = SpinLock::INIT;

        for _ in 0..100 {
            lock.lock();
            thread::sleep(Duration::from_millis(10));
            lock.unlock();
        }
    }

    #[test]
    fn spinlock_loop_multi() {
        // Every instance occurs in a thread now.

        let lock = Arc::new(SpinLock::INIT);
        let mut threads = Vec::new();

        for _ in 0..100 {
            let lock = lock.clone();
            let t = thread::spawn(move || {
                lock.lock();
                thread::sleep(Duration::from_millis(10));
                lock.unlock();
            });

            threads.push(t);
        }

        for t in threads {
            t.join().unwrap();
        }
    }

    #[test]
    fn spinlock() {
        let lock = Arc::new(SpinLock::INIT);
        let touched = Arc::new(Mutex::new(false));

        lock.lock();

        let lock_t = lock.clone();
        let touched_t = touched.clone();

        thread::spawn(move || {
            *touched_t.lock().unwrap() = true;
            lock_t.unlock();
        });

        lock.lock();
        assert_eq!(true, *touched.lock().unwrap());
    }

    #[test]
    fn spinlock_sleep() {
        let lock = Arc::new(SpinLock::INIT);
        let touched = Arc::new(Mutex::new(false));

        lock.lock();

        let lock_t = lock.clone();
        let touched_t = touched.clone();

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            *touched_t.lock().unwrap() = true;
            lock_t.unlock();
        });

        // This lock should block here until the spinlock is unlocked.
        lock.lock();
        assert_eq!(true, *touched.lock().unwrap());
    }
}
