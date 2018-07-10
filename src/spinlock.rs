use std::sync::atomic::{AtomicBool, Ordering};

pub struct SpinLock(AtomicBool);

impl SpinLock {
    pub const INIT: SpinLock = SpinLock(AtomicBool::new(false));

    pub fn lock(&self) {
        while self.0.swap(true, Ordering::SeqCst) {}
    }

    pub fn unlock(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
