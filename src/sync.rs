use core::arch::asm;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A spinlock that also disables interrupts while held.
///
/// This kernel is single-core, so the only source of real concurrency is an
/// interrupt handler running while the main flow holds the lock. Disabling
/// interrupts for the lock's duration rules that out without needing a
/// general-purpose recursive lock.
pub struct Mutex<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(data: T) -> Self {
        Mutex {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        let were_enabled = interrupts_enabled();
        unsafe { asm!("cli", options(nomem, nostack)) };
        while self.locked.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        MutexGuard {
            mutex: self,
            restore_interrupts: were_enabled,
        }
    }
}

#[inline]
fn interrupts_enabled() -> bool {
    let flags: u32;
    unsafe {
        asm!("pushfd; mov {0}, [esp]; popfd", out(reg) flags, options(nomem));
    }
    flags & (1 << 9) != 0
}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
    restore_interrupts: bool,
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        if self.restore_interrupts {
            unsafe { asm!("sti", options(nomem, nostack)) };
        }
    }
}
