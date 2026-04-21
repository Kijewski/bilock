#![allow(clippy::unwrap_used)] // it's okay to `unwrap()` in test

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::sync::{Barrier, BarrierWaitResult};
use std::thread::{sleep, spawn, yield_now};

use crate::{Bilock, Guard, OwnedGuard};

#[allow(dead_code)] // it's only there to ensure an assumption
trait ExpectedToBeUnpin: Unpin {}

impl<T> ExpectedToBeUnpin for Bilock<T> {}
impl<T> ExpectedToBeUnpin for Guard<'_, T> {}
impl<T> ExpectedToBeUnpin for OwnedGuard<T> {}

#[test]
fn new_returns_two_handles() {
    let (mut left, mut right) = Bilock::new(42);

    assert_eq!(*left.lock(), 42);
    assert_eq!(right.try_lock().as_deref(), Some(&42));
}

#[test]
fn shared_value_survives_until_last_drop() {
    struct DropCounter {
        drops: Arc<AtomicUsize>,
    }

    impl Drop for DropCounter {
        fn drop(&mut self) {
            let _: usize = self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let value = DropCounter {
        drops: drops.clone(),
    };

    let (left, right) = Bilock::new(value);
    drop(left);
    assert_eq!(drops.load(Ordering::SeqCst), 0);

    drop(right);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn try_lock_returns_none_while_other_guard_exists() {
    let (mut left, mut right) = Bilock::new(0);
    let _guard = left.lock();

    assert!(right.try_lock().is_none());
}

#[test]
fn lock_works_after_other_handle_dropped_while_guard_held() {
    let (mut left, right) = Bilock::new(7);
    let guard = left.lock();
    drop(right);
    drop(guard);

    let guard = left.lock();
    assert_eq!(*guard, 7);
}

#[test]
fn lock_is_exclusive_across_threads() {
    let (mut left, mut right) = Bilock::new(0);
    let started = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));

    let handle = spawn({
        let finished = Arc::clone(&finished);
        let started = Arc::clone(&started);
        move || {
            let mut guard = right.lock();
            started.store(true, Ordering::SeqCst);
            while !finished.load(Ordering::SeqCst) {
                yield_now();
            }
            *guard = 42;
        }
    });

    while !started.load(Ordering::SeqCst) {
        yield_now();
    }

    assert!(left.try_lock().is_none());
    finished.store(true, Ordering::SeqCst);
    handle.join().unwrap();

    assert_eq!(*left.lock(), 42);
}

#[test]
fn repeated_lock_unlock_across_threads_is_sound() {
    #[cfg(not(miri))]
    const ITERATIONS: usize = 10_000;
    #[cfg(miri)]
    const ITERATIONS: usize = 1000;

    let barrier = Arc::new(Barrier::new(2));
    let (mut left, mut right) = Bilock::new(0usize);

    let left = spawn({
        let barrier = Arc::clone(&barrier);
        move || {
            let _: BarrierWaitResult = barrier.wait();
            for _ in 0..ITERATIONS {
                let mut guard = left.lock();
                *guard += 1;
            }
            left
        }
    });

    let right = spawn(move || {
        let _: BarrierWaitResult = barrier.wait();
        for _ in 0..ITERATIONS {
            let mut guard = right.lock();
            *guard += 1;
        }
        right
    });

    let mut left = left.join().unwrap();
    let mut right = right.join().unwrap();
    assert_eq!(left.try_lock().as_deref(), Some(&(ITERATIONS * 2)));
    assert_eq!(right.try_lock().as_deref(), Some(&(ITERATIONS * 2)));
}

#[test]
fn owned_lock_unlock_returns_handle() {
    let (left, right) = Bilock::new(13);
    let owned = left.owned_lock();
    assert_eq!(*owned, 13);

    let mut left = OwnedGuard::unlock(owned);
    assert_eq!(*left.lock(), 13);
    drop(right);
    drop(left);
}

#[test]
fn join_returns_value_for_same_pair() {
    struct DropCounter {
        drops: Arc<AtomicUsize>,
    }

    impl Drop for DropCounter {
        fn drop(&mut self) {
            let _: usize = self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let value = DropCounter {
        drops: drops.clone(),
    };

    let (left, right) = Bilock::new(value);
    let owned = left.owned_lock();
    let value = Bilock::join(owned, right).unwrap();

    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(value);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn join_returns_err_for_different_pairs() {
    let (l1, _r1) = Bilock::new(1);
    let (l2, _r2) = Bilock::new(2);

    let owned = l1.owned_lock();
    let (owned, mut l2) = Bilock::join(owned, l2).unwrap_err();

    assert_eq!(*owned, 1);
    assert_eq!(*l2.lock(), 2);
}

#[test]
fn try_owned_lock_fails_when_locked() {
    let (mut left, right) = Bilock::new(0);
    let _guard = left.lock();

    assert!(right.try_owned_lock().is_err());
}

#[test]
fn owned_guard_drop_releases_lock() {
    let (left, mut right) = Bilock::new(0);
    let _owned = left.owned_lock();

    assert!(right.try_lock().is_none());
}

#[test]
fn formatting_is_sane() {
    let (mut left, right) = Bilock::new(0);
    let owned = left.lock();

    let owned = std::format!("{owned:?}");
    let right = std::format!("{right:?}");
    assert_eq!(
        owned.split_once(' ').unwrap().1,
        right.split_once(' ').unwrap().1,
    );
}

#[test]
#[should_panic = "bilock timeout test intentionally terminated"]
fn lock_deadlocks_on_same_thread_reentry() {
    with_timeout(|barrier| {
        let (mut left, mut right) = Bilock::new(0);
        let _guard = left.lock();
        let _: BarrierWaitResult = barrier.wait();
        let _guard2 = right.lock();
    });
}

#[test]
#[should_panic = "bilock timeout test intentionally terminated"]
fn owned_lock_deadlocks_on_same_thread_reentry() {
    with_timeout(|barrier| {
        let (left, right) = Bilock::new(0);
        let _owned = left.owned_lock();
        let _: BarrierWaitResult = barrier.wait();
        let _owned2 = right.owned_lock();
    });
}

#[track_caller]
fn with_timeout(f: fn(&Barrier)) {
    let barrier = Arc::new(Barrier::new(2));

    let handle = spawn({
        let barrier = Arc::clone(&barrier);
        move || f(&barrier)
    });

    let _: BarrierWaitResult = barrier.wait();
    sleep(Duration::from_millis(100));
    if !handle.is_finished() {
        std::panic!("bilock timeout test intentionally terminated");
    } else if let Err(err) = handle.join() {
        std::panic::resume_unwind(err);
    } else {
        unreachable!("Did not deadlock?");
    }
}
