// SPDX-FileCopyrightText: 2026 René Kijewski <crates.io@k6i.de>
// SPDX-License-Identifier: ISC OR MIT OR Apache-2.0

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! # Bilock: a minimal spin-lock based two-handle mutex pair for `no_std` Rust
//!
//! [![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/Kijewski/bilock/ci.yml?branch=main&style=flat-square&logo=github&logoColor=white "GitHub Workflow Status")](https://github.com/Kijewski/bilock/actions/workflows/ci.yml)
//! [![Crates.io](https://img.shields.io/crates/v/bilock?logo=rust&style=flat-square "Crates.io")](https://crates.io/crates/bilock)
//! [![docs.rs](https://img.shields.io/docsrs/bilock?logo=docsdotrs&style=flat-square&logoColor=white "docs.rs")](https://docs.rs/bilock/)
//!
//! [`Bilock::new()`] provides two linked handles that share ownership of the same
//! guarded value. A lock is held by either a temporary [`Guard`] or an
//! [`OwnedGuard`], and the underlying value is released once both handles are
//! dropped.
//!
//! The library employs [spin loops](hint::spin_loop) to wait for the lock,
//! so it is intended for short critical sections only.
//!
//! # Example
//!
//! ```
//! use bilock::Bilock;
//!
//! let (mut left, mut right) = Bilock::new(42);
//! let guard = left.lock();
//! assert_eq!(*guard, 42);
//! drop(guard);
//!
//! let mut guard = right.lock();
//! *guard = 4711;
//! assert_eq!(*guard, 4711);
//! ```
//!
//! ## License
//!
//! This project is tri-licensed under <tt>ISC OR MIT OR Apache-2.0</tt>.
//! Contributions must be licensed under the same terms.
//! Users may follow any one of these licenses, or all of them.
//!
//! See the individual license texts at
//! * <https://spdx.org/licenses/ISC.html>,
//! * <https://spdx.org/licenses/MIT.html>, and
//! * <https://spdx.org/licenses/Apache-2.0.html>.

extern crate alloc;
#[cfg(any(doc, test))]
extern crate std;

#[cfg(test)]
mod tests;

use alloc::boxed::Box;
use core::sync::atomic;
use core::{cell, fmt, hint, marker, mem, ops, ptr};

use crate::private::BilockLike as _;

/// This struct behaves like <code>[Arc](std::sync::Arc)&lt;[Mutex](std::sync::Mutex)&lt;T&gt;&gt;</code>,
/// but it is neither [`Clone`] nor [`Default`].
///
/// The library employs [spin loops](hint::spin_loop) to wait for the lock,
/// so it is intended for short critical sections only.
///
/// # Example
///
/// ```
/// use bilock::Bilock;
///
/// let (mut a, mut b) = Bilock::new(42);
/// assert_eq!(*a.lock(), 42);
/// assert_eq!(*b.lock(), 42);
/// ```
pub struct Bilock<T> {
    ptr: ptr::NonNull<Inner<T>>,
}

/// Guard that holds the lock and unlocks on drop.
///
/// # Example
///
/// ```
/// use bilock::Bilock;
///
/// let (mut a, _) = Bilock::new(1);
/// let guard = a.lock();
/// assert_eq!(*guard, 1);
/// ```
pub struct Guard<'a, T> {
    ptr: ptr::NonNull<Inner<T>>,
    _bilock: marker::PhantomData<&'a mut Bilock<T>>,
}

/// Owned lock guard holding the inner value until dropped or unlocked.
///
/// # Example
///
/// ```
/// use bilock::{Bilock, OwnedGuard};
///
/// let (a, _) = Bilock::new(1);
/// let owned = a.owned_lock();
/// assert_eq!(*owned, 1);
/// ```
pub struct OwnedGuard<T> {
    ptr: ptr::NonNull<Inner<T>>,
}

struct Inner<T> {
    value: cell::UnsafeCell<T>,
    state: atomic::AtomicU8,
}

/// Trait implemented by types that belong to the same [`Bilock`] pair.
///
/// # Example
///
/// ```
/// use bilock::{BilockLike, Bilock};
///
/// let (a, b) = Bilock::new(0);
/// assert!(Bilock::ptr_eq(&a, &b));
/// ```
pub trait BilockLike: private::BilockLike {
    /// True if `left` and `right` belong to the same [`Bilock`] pair.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::{BilockLike, Bilock};
    ///
    /// let (a, b) = Bilock::new(0);
    /// assert!(Bilock::ptr_eq(&a, &b));
    /// ```
    #[inline]
    fn ptr_eq(left: &Self, right: &impl BilockLike) -> bool {
        ptr::addr_eq(left.state(), right.state())
    }

    /// Returns `true` if the paired handle is still alive.
    ///
    /// This can be used to detect whether the other side of a [`Bilock`] pair
    /// has already been dropped.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::{Bilock, BilockLike};
    ///
    /// let (left, right) = Bilock::new(0);
    /// assert!(left.other_side_alive());
    /// drop(right);
    /// assert!(!left.other_side_alive());
    /// ```
    #[inline]
    fn other_side_alive(&self) -> bool {
        self.state().load(atomic::Ordering::Acquire) & ALIVE_FLAG == ALIVE_FLAG
    }
}

impl<T> BilockLike for Bilock<T> {}
impl<T> BilockLike for Guard<'_, T> {}
impl<T> BilockLike for OwnedGuard<T> {}

unsafe impl<T: Send + Sync> Send for Bilock<T> {}
unsafe impl<T: Send + Sync> Sync for Bilock<T> {}
unsafe impl<T: Send + Sync> Send for Guard<'_, T> {}
unsafe impl<T: Send + Sync> Sync for Guard<'_, T> {}
unsafe impl<T: Send + Sync> Send for OwnedGuard<T> {}
unsafe impl<T: Send + Sync> Sync for OwnedGuard<T> {}

impl<T> fmt::Debug for Bilock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_ptr(f, "Bilock", self.value())
    }
}

impl<T> fmt::Debug for Guard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_ptr(f, "Guard", self.value())
    }
}

impl<T> fmt::Debug for OwnedGuard<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_ptr(f, "OwnedGuard", self.value())
    }
}

#[inline]
fn debug_ptr(f: &mut fmt::Formatter<'_>, name: &str, ptr: *const ()) -> Result<(), fmt::Error> {
    f.debug_struct(name).field("ptr", &ptr).finish()
}

impl<T> Bilock<T> {
    /// Return two instances, and once both are dropped, the contained value is freed.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (mut a, mut b) = Bilock::new(42);
    /// assert_eq!(*a.lock(), 42);
    /// assert_eq!(*b.try_lock().unwrap(), 42);
    /// ```
    #[inline]
    pub fn new(value: T) -> (Self, Self) {
        // SAFETY: this is just a placement new
        let inner = unsafe {
            let mut inner = Box::<Inner<T>>::new_uninit();
            ptr::write(
                &raw mut (*inner.as_mut_ptr()).value,
                cell::UnsafeCell::new(value),
            );
            ptr::write(
                &raw mut (*inner.as_mut_ptr()).state,
                atomic::AtomicU8::new(ALIVE_FLAG | UNLOCKED_FLAG),
            );
            inner.assume_init()
        };

        // SAFETY: `Box::into_raw` returns a non-null pointer to a valid `Inner<T>`.
        let ptr = unsafe { ptr::NonNull::new_unchecked(Box::into_raw(inner)) };

        (Self { ptr }, Self { ptr })
    }

    /// Consumes `self`, and blocks until the contained value can be acquired.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (a, _) = Bilock::new(3);
    /// let owned = a.owned_lock();
    /// assert_eq!(*owned, 3);
    /// ```
    ///
    /// # Usage Warning
    ///
    /// <div class="warning">
    ///
    /// Calling [`.lock()`][Bilock::lock()] or [`.owned_lock()`][Bilock::owned_lock()] in the
    /// same thread that already holds a lock for the paired [`Bilock`] instance will cause a
    /// **deadlock**.
    ///
    /// ```no_run
    /// use bilock::Bilock;
    ///
    /// let (a, b) = Bilock::new(3);
    /// let owned_a = a.owned_lock();
    /// // Lock already held for `a` → deadlock:
    /// let owned_b = b.owned_lock();
    /// unreachable!("This line is never reached!");
    /// ```
    ///
    /// </div>
    #[inline]
    pub fn owned_lock(mut self) -> OwnedGuard<T> {
        // SAFETY: `self.lock()` returns a valid `Guard` holding the lock on the same inner pointer,
        // and `self` is forgotten immediately so the original handle is never dropped.
        let guard = unsafe { Guard::into_owned(self.lock()) };
        mem::forget(self);
        guard
    }

    /// Blocks until the contained value can be acquired.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (mut a, _) = Bilock::new(4);
    /// let guard = a.lock();
    /// assert_eq!(*guard, 4);
    /// ```
    ///
    /// # Usage Warning
    ///
    /// <div class="warning">
    ///
    /// Calling [`.lock()`][Bilock::lock()] or [`.owned_lock()`][Bilock::owned_lock()] in the
    /// same thread that already holds a lock for the paired [`Bilock`] instance will cause a
    /// **deadlock**.
    ///
    /// ```no_run
    /// use bilock::Bilock;
    ///
    /// let (mut a, mut b) = Bilock::new(3);
    /// let guard_a = a.lock();
    /// // Lock already held for `a` → deadlock:
    /// let guard_b = b.lock();
    /// unreachable!("This line is never reached!");
    /// ```
    ///
    /// </div>
    pub fn lock(&mut self) -> Guard<'_, T> {
        let mut this = self;
        loop {
            this = match this.do_try_lock() {
                Ok(guard) => return guard,
                Err(this) => this,
            };
            hint::spin_loop();
        }
    }

    /// If currently unlocked, an owned lock is acquired.
    ///
    /// # Errors
    ///
    /// If a lock is held, then `self` is returned back.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (a, _) = Bilock::new(5);
    /// let owned = a.try_owned_lock().unwrap();
    /// assert_eq!(*owned, 5);
    /// ```
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (a, b) = Bilock::new(5);
    /// let owned_a = a.try_owned_lock().unwrap();
    /// assert_eq!(*owned_a, 5);
    /// let owned_b = b.try_owned_lock();
    /// assert!(owned_b.is_err());
    /// ```
    #[inline]
    pub fn try_owned_lock(mut self) -> Result<OwnedGuard<T>, Self> {
        let Some(guard) = self.try_lock() else {
            return Err(self);
        };

        // SAFETY: the guard holds the lock and we'll forget the Bilock.
        let guard = unsafe { Guard::into_owned(guard) };
        mem::forget(self);
        Ok(guard)
    }

    /// If currently unlocked, a lock is acquired.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (mut a, _) = Bilock::new(6);
    /// assert!(a.try_lock().is_some());
    /// ```
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (mut a, mut b) = Bilock::new(6);
    /// let guard_a = a.try_lock();
    /// assert_eq!(guard_a.as_deref(), Some(&6));
    /// let guard_b = b.try_lock();
    /// assert_eq!(guard_b.as_deref(), None);
    /// ```
    #[inline]
    pub fn try_lock(&mut self) -> Option<Guard<'_, T>> {
        self.do_try_lock().ok()
    }

    /// Same as [`Bilock::try_lock()`], but returns `&mut Self` on error. That's useful in loops.
    fn do_try_lock(&mut self) -> Result<Guard<'_, T>, &mut Self> {
        let mut old_state = self.state().load(atomic::Ordering::Acquire);
        loop {
            if old_state & UNLOCKED_FLAG != UNLOCKED_FLAG {
                return Err(self);
            } else if let Err(new_state) = self.state().compare_exchange_weak(
                old_state,
                old_state & !UNLOCKED_FLAG,
                atomic::Ordering::Acquire,
                atomic::Ordering::Relaxed,
            ) {
                old_state = new_state;
                hint::spin_loop();
            } else {
                return Ok(Guard {
                    ptr: self.ptr,
                    _bilock: marker::PhantomData,
                });
            }
        }
    }

    /// If `guard` and `other` were created using using the same [`Bilock::new()`] call,
    /// then their contained value is returned.
    ///
    /// # Errors
    ///
    /// If `guard` and `other` are not the result of the same [`Bilock::new()`] call,
    /// then they are returned unchanged.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (a, b) = Bilock::new(7);
    /// let owned = a.owned_lock();
    /// let value = Bilock::join(owned, b).unwrap();
    /// assert_eq!(value, 7);
    /// ```
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (a, _) = Bilock::new(8);
    /// let (_, b) = Bilock::new(9);
    /// let owned = a.owned_lock();
    /// assert!(Bilock::join(owned, b).is_err());
    /// ```
    #[inline]
    pub fn join(guard: OwnedGuard<T>, other: Self) -> Result<T, (OwnedGuard<T>, Self)> {
        if guard.ptr == other.ptr {
            // SAFETY: we just checked that `guard` and `other` are paired.
            Ok(unsafe { Self::join_unchecked(guard, other) })
        } else {
            Err((guard, other))
        }
    }

    /// Consume an owned lock guard and the original mutex handle, returning the inner value
    /// without verifying that they belong to the same mutex pair.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `guard` and `other` were created by the same
    /// [`Bilock::new()`] call.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (a, b) = Bilock::new(8);
    /// let owned = a.owned_lock();
    /// let value = unsafe { Bilock::join_unchecked(owned, b) };
    /// assert_eq!(value, 8);
    /// ```
    #[inline]
    pub unsafe fn join_unchecked(guard: OwnedGuard<T>, other: Self) -> T {
        drop(other);
        // SAFETY: We ensured that `guard` and `other` are paired. After dropping `other`,
        // we can be sure that `!guard.other_side_alive()`.
        unsafe { Self::into_inner_unchecked(guard) }
    }

    /// Consume an [`OwnedGuard`] and return the inner value if the other
    /// side of the [`Bilock`] pair has already been dropped.
    ///
    /// # Errors
    ///
    /// If the paired [`Bilock`] is still alive, the `guard` is returned unchanged.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (a, b) = Bilock::new(1);
    /// let owned = a.owned_lock();
    /// drop(b);
    /// assert_eq!(Bilock::into_inner(owned).unwrap(), 1);
    /// ```
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (a, b) = Bilock::new(1);
    /// let owned = a.owned_lock();
    /// assert!(Bilock::into_inner(owned).is_err());
    /// ```
    #[inline]
    pub fn into_inner(guard: OwnedGuard<T>) -> Result<T, OwnedGuard<T>> {
        if guard.state().load(atomic::Ordering::Acquire) & ALIVE_FLAG != ALIVE_FLAG {
            // SAFETY: we just checked that `guard`'s other side was dropped.
            Ok(unsafe { Self::into_inner_unchecked(guard) })
        } else {
            Err(guard)
        }
    }

    /// Consume an [`OwnedGuard`] and return the inner value without checking
    /// whether the paired handle is still alive.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the paired handle was dropped.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::Bilock;
    ///
    /// let (a, b) = Bilock::new(1);
    /// let owned = a.owned_lock();
    /// drop(b);
    /// let value = unsafe { Bilock::into_inner_unchecked(owned) };
    /// assert_eq!(value, 1);
    /// ```
    #[inline]
    pub unsafe fn into_inner_unchecked(guard: OwnedGuard<T>) -> T {
        // SAFETY: `guard.ptr` points to a valid `Inner<T>`.
        let inner = unsafe { Box::from_raw(guard.ptr.as_ptr()) };
        mem::forget(guard);
        inner.value.into_inner()
    }
}

impl<T> Guard<'_, T> {
    /// Convert a temporary lock guard into an owned guard without releasing the lock.
    ///
    /// # Safety
    ///
    /// The caller must ensure the original [`Bilock`] is not used
    /// (this includes [dropping][Drop]!) after conversion, e.g. by using [`std::mem::forget()`].
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::{Bilock, Guard};
    ///
    /// let (mut a, _) = Bilock::new(9);
    /// let guard = a.lock();
    /// let owned = unsafe { Guard::into_owned(guard) };
    /// std::mem::forget(a);
    /// assert_eq!(*owned, 9);
    /// ```
    #[inline]
    pub unsafe fn into_owned(guard: Guard<'_, T>) -> OwnedGuard<T> {
        let owned_guard = OwnedGuard { ptr: guard.ptr };
        mem::forget(guard);
        owned_guard
    }

    /// Release the lock by dropping the guard explicitly.
    ///
    /// This is the same as [dropping][Drop] the guard.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::{Bilock, Guard};
    ///
    /// let (mut a, _) = Bilock::new(10);
    /// let guard = a.lock();
    /// Guard::unlock(guard);
    /// assert!(a.try_lock().is_some());
    /// ```
    #[inline]
    pub fn unlock(guard: Self) {
        drop(guard);
    }
}

impl<T> OwnedGuard<T> {
    /// Release the lock and return ownership of the original mutex handle.
    ///
    /// # Example
    ///
    /// ```
    /// use bilock::{Bilock, OwnedGuard};
    ///
    /// let (mut a, _) = Bilock::new(11);
    /// let owned = a.owned_lock();
    /// let mut a = OwnedGuard::unlock(owned);
    /// assert_eq!(*a.lock(), 11);
    /// ```
    #[inline]
    pub fn unlock(guard: Self) -> Bilock<T> {
        let _: u8 = guard
            .state()
            .fetch_or(UNLOCKED_FLAG, atomic::Ordering::Release);
        let bilock = Bilock { ptr: guard.ptr };
        mem::forget(guard);
        bilock
    }
}

impl<T> ops::Deref for Guard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: `self.ptr` points to a valid `Inner<T>` and the guard holds the lock.
        unsafe { &*(*self.ptr.as_ptr()).value.get() }
    }
}

impl<T> ops::DerefMut for Guard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: `self.ptr` points to a valid `Inner<T>` and the guard holds the lock.
        unsafe { (*self.ptr.as_ptr()).value.get_mut() }
    }
}

impl<T> ops::Deref for OwnedGuard<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: `self.ptr` points to a valid `Inner<T>` and the guard holds the lock.
        unsafe { &*(*self.ptr.as_ptr()).value.get() }
    }
}

impl<T> ops::DerefMut for OwnedGuard<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: `self.ptr` points to a valid `Inner<T>` and the guard holds the lock.
        unsafe { (*self.ptr.as_ptr()).value.get_mut() }
    }
}

impl<T> Drop for Bilock<T> {
    fn drop(&mut self) {
        let old_state = self
            .state()
            .fetch_and(!ALIVE_FLAG, atomic::Ordering::AcqRel);
        if old_state & ALIVE_FLAG != ALIVE_FLAG {
            // SAFETY: `self.ptr` points to a valid `Inner<T>`, and its last reference was dropped.
            drop(unsafe { Box::from_raw(self.ptr.as_ptr()) });
        }
    }
}

impl<T> Drop for Guard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        let _: u8 = self
            .state()
            .fetch_or(UNLOCKED_FLAG, atomic::Ordering::Release);
    }
}

impl<T> Drop for OwnedGuard<T> {
    fn drop(&mut self) {
        drop(Self::unlock(Self { ptr: self.ptr }));
    }
}

mod private {
    use super::*;

    pub trait BilockLike {
        fn state(&self) -> &atomic::AtomicU8;
        fn value(&self) -> *const ();
    }

    impl<T> BilockLike for Bilock<T> {
        #[inline]
        fn state(&self) -> &atomic::AtomicU8 {
            // SAFETY: `self.ptr` points to a valid `Inner<T>`.
            unsafe { &(*self.ptr.as_ptr()).state }
        }

        #[inline]
        fn value(&self) -> *const () {
            // SAFETY: `self.ptr` points to a valid `Inner<T>`.
            unsafe { ptr::addr_of!((*self.ptr.as_ptr()).value).cast() }
        }
    }

    impl<T> BilockLike for Guard<'_, T> {
        #[inline]
        fn state(&self) -> &atomic::AtomicU8 {
            // SAFETY: `self.ptr` points to a valid `Inner<T>`.
            unsafe { &(*self.ptr.as_ptr()).state }
        }

        #[inline]
        fn value(&self) -> *const () {
            // SAFETY: `self.ptr` points to a valid `Inner<T>`.
            unsafe { ptr::addr_of!((*self.ptr.as_ptr()).value).cast() }
        }
    }

    impl<T> BilockLike for OwnedGuard<T> {
        #[inline]
        fn state(&self) -> &atomic::AtomicU8 {
            // SAFETY: `self.ptr` points to a valid `Inner<T>`.
            unsafe { &(*self.ptr.as_ptr()).state }
        }

        #[inline]
        fn value(&self) -> *const () {
            // SAFETY: `self.ptr` points to a valid `Inner<T>`.
            unsafe { ptr::addr_of!((*self.ptr.as_ptr()).value).cast() }
        }
    }
}

/// Both sides of the [`Bilock`] are alive, i.e. neither side was dropped.
const ALIVE_FLAG: u8 = 1 << 0;
/// The [`Bilock`] is unlocked, i.e. a guard can be acquired.
const UNLOCKED_FLAG: u8 = 1 << 1;
