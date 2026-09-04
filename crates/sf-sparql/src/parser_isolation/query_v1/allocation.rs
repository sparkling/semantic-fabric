use std::alloc::{alloc, Layout};

use super::error::QueryWireError;

/// Allocates one recursive AST node without invoking the infallible OOM path.
pub(super) fn try_box<T>(value: T) -> Result<Box<T>, QueryWireError> {
    let layout = Layout::new::<T>();
    if layout.size() == 0 {
        return Ok(Box::new(value));
    }

    // SAFETY: `alloc` receives the exact layout for `T`. A non-null result is
    // aligned for that layout, initialized once with `value`, and immediately
    // transferred to `Box`, which will deallocate it with the same global
    // allocator and layout. Null is handled as a closed allocation failure.
    let pointer = unsafe { alloc(layout).cast::<T>() };
    if pointer.is_null() {
        return Err(QueryWireError::AllocationFailed);
    }
    // SAFETY: the successful allocation above is valid and uniquely owned.
    unsafe {
        pointer.write(value);
        Ok(Box::from_raw(pointer))
    }
}
