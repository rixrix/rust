// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use prelude::v1::*;

use cell::UnsafeCell;
use marker;
use ops::{Deref, DerefMut};
use sync::poison::{self, TryLockError, TryLockResult, LockResult};
use sys::mutex as sys;
use fmt;

/// A re-entrant mutual exclusion
///
/// This mutex will block *other* threads waiting for the lock to become available. The thread
/// which has already locked the mutex can lock it multiple times without blocking, preventing a
/// common source of deadlocks.
///
/// # Examples
///
/// ```
/// use std::sync::ReentrantMutex;
///
/// fn locking_mod(data: ReentrantMutex<i32>) {
///     let mut data = data.lock().unwrap();
///     *data += 1;
/// }
///
/// let data = ReentrantMutex::new(0);
/// {
/// // We take the lock twice here. It won’t deadlock.
/// let lock = data.lock();
/// locking_mod(data);
/// }
/// ```
#[unstable(feature = "reentrant_mutex", reason = "new API")]
pub struct ReentrantMutex<T> {
    inner: Box<sys::ReentrantMutex>,
    poison: poison::Flag,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for ReentrantMutex<T> {}
unsafe impl<T: Send> Sync for ReentrantMutex<T> {}


/// An RAII implementation of a "scoped lock" of a mutex. When this structure is
/// dropped (falls out of scope), the lock will be unlocked.
///
/// The data protected by the mutex can be accessed through this guard via its
/// Deref and DerefMut implementations
#[must_use]
#[unstable(feature = "reentrant_mutex", reason = "new API")]
pub struct ReentrantMutexGuard<'a, T: 'a> {
    // funny underscores due to how Deref/DerefMut currently work (they
    // disregard field privacy).
    __lock: &'a ReentrantMutex<T>,
    __poison: poison::Guard,
}

impl<'a, T> !marker::Send for ReentrantMutexGuard<'a, T> {}


#[unstable(feature = "reentrant_mutex", reason = "new API")]
impl<T> ReentrantMutex<T> {
    /// Creates a new reentrant mutex in an unlocked state.
    pub fn new(t: T) -> ReentrantMutex<T> {
        ReentrantMutex {
            inner: box unsafe { sys::ReentrantMutex::new() },
            poison: poison::FLAG_INIT,
            data: UnsafeCell::new(t),
        }
    }

    /// Acquires a mutex, blocking the current thread until it is able to do so.
    ///
    /// This function will block the caller until it is available to acquire the mutex.
    /// Upon returning, the thread is the only thread with the mutex held. When the thread
    /// calling this method already holds the lock, the call shall succeed without
    /// blocking.
    ///
    /// # Failure
    ///
    /// If another user of this mutex panicked while holding the mutex, then
    /// this call will return failure if the mutex would otherwise be
    /// acquired.
    pub fn lock(&self) -> LockResult<ReentrantMutexGuard<T>> {
        unsafe { self.inner.lock() }
        ReentrantMutexGuard::new(&self)
    }

    /// Attempts to acquire this lock.
    ///
    /// If the lock could not be acquired at this time, then `Err` is returned.
    /// Otherwise, an RAII guard is returned.
    ///
    /// This function does not block.
    ///
    /// # Failure
    ///
    /// If another user of this mutex panicked while holding the mutex, then
    /// this call will return failure if the mutex would otherwise be
    /// acquired.
    pub fn try_lock(&self) -> TryLockResult<ReentrantMutexGuard<T>> {
        if unsafe { self.inner.try_lock() } {
            Ok(try!(ReentrantMutexGuard::new(&self)))
        } else {
            Err(TryLockError::WouldBlock)
        }
    }

    /// Determine whether the lock is poisoned.
    ///
    /// If another thread is active, the lock can still become poisoned at any
    /// time.  You should not trust a `false` value for program correctness
    /// without additional synchronization.
    #[inline]
    pub fn is_poisoned(&self) -> bool {
        self.poison.get()
    }
}

#[unsafe_destructor]
#[unstable(feature = "reentrant_mutex", reason = "new API")]
impl<T> Drop for ReentrantMutex<T> {
    fn drop(&mut self) {
        // This is actually safe b/c we know that there is no further usage of
        // this mutex (it's up to the user to arrange for a mutex to get
        // dropped, that's not our job)
        unsafe { self.inner.destroy() }
    }
}

#[unstable(feature = "reentrant_mutex", reason = "new API")]
impl<T: fmt::Debug + 'static> fmt::Debug for ReentrantMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.try_lock() {
            Ok(guard) => write!(f, "ReentrantMutex {{ data: {:?} }}", *guard),
            Err(TryLockError::Poisoned(err)) => {
                write!(f, "ReentrantMutex {{ data: Poisoned({:?}) }}", **err.get_ref())
            },
            Err(TryLockError::WouldBlock) => write!(f, "ReentrantMutex {{ <locked> }}")
        }
    }
}

#[unstable(feature = "reentrant_mutex", reason = "new API")]
impl<'mutex, T> ReentrantMutexGuard<'mutex, T> {
    fn new(lock: &'mutex ReentrantMutex<T>)
            -> LockResult<ReentrantMutexGuard<'mutex, T>> {
        poison::map_result(lock.poison.borrow(), |guard| {
            ReentrantMutexGuard {
                __lock: lock,
                __poison: guard,
            }
        })
    }
}

#[unstable(feature = "reentrant_mutex", reason = "new API")]
impl<'mutex, T> Deref for ReentrantMutexGuard<'mutex, T> {
    type Target = T;

    fn deref<'a>(&'a self) -> &'a T {
        unsafe { &*self.__lock.data.get() }
    }
}

#[unstable(feature = "reentrant_mutex", reason = "new API")]
impl<'mutex, T> DerefMut for ReentrantMutexGuard<'mutex, T> {
    fn deref_mut<'a>(&'a mut self) -> &'a mut T {
        unsafe { &mut *self.__lock.data.get() }
    }
}

#[unsafe_destructor]
#[unstable(feature = "reentrant_mutex", reason = "new API")]
impl<'a, T> Drop for ReentrantMutexGuard<'a, T> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            self.__lock.poison.done(&self.__poison);
            self.__lock.inner.unlock();
        }
    }
}


#[cfg(test)]
mod test {
    use prelude::v1::*;
    use sync::ReentrantMutex;
    use thread;

    #[test]
    fn smoke(){
        let m = ReentrantMutex::new(());
        {
            let a = m.lock().unwrap();
            {
                let b = m.lock().unwrap();
                {
                    let c = m.lock().unwrap();
                    assert_eq!(*c, ());
                }
                assert_eq!(*b, ());
            }
            assert_eq!(*a, ());
        }
    }

    #[test]
    fn is_mutex(){
        let m = ReentrantMutex::new(0);
        let lock = m.lock().unwrap();
        let handle = thread::scoped(|| {
            let lock = m.lock().unwrap();
            assert_eq!(*lock, 4950);
        });
        for i in 0..100 {
            let mut lock = m.lock().unwrap();
            *lock += i;
        }
        drop(lock);
        drop(handle);
    }

    #[test]
    fn trylock_works(){
        let m = ReentrantMutex::new(());
        let lock = m.try_lock().unwrap();
        let lock2 = m.try_lock().unwrap();
        {
            thread::scoped(|| {
                let lock = m.try_lock();
                assert!(lock.is_err());
            });
        }
        let lock3 = m.try_lock().unwrap();
    }
}
