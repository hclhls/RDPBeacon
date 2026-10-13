use std::ptr::NonNull;

// Native callbacks retain this raw pointer. Keep the allocation out of a Box
// until callbacks have been detached, so moving the owner never retags it.
pub(super) struct CallbackState<T> {
    pointer: NonNull<T>,
}

impl<T> CallbackState<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            pointer: NonNull::new(Box::into_raw(Box::new(value)))
                .expect("Box allocation is non-null"),
        }
    }
    pub(super) fn as_ptr(&self) -> *mut T {
        self.pointer.as_ptr()
    }
}
impl<T> Drop for CallbackState<T> {
    fn drop(&mut self) {
        // The native owner must destroy its window before dropping this field.
        unsafe {
            drop(Box::from_raw(self.pointer.as_ptr()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct State {
        value: usize,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for State {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn callback_pointer_survives_owner_move_and_is_freed_once() {
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = CallbackState::new(State {
            value: 1,
            drops: drops.clone(),
        });
        let registered = owner.as_ptr();
        let moved = owner;
        unsafe {
            (*moved.as_ptr()).value = 2;
            assert_eq!((*registered).value, 2);
        }
        drop(moved);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }
}
