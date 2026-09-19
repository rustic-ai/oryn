//! Construct child continuations in a separate native frame. Boxing only at the
//! call site still reserves the entire by-value result in a debug poll frame.
//! The closure captures references; ownership transfers only after construction.
#[inline(never)]
pub(super) fn boxed<T>(make: impl FnOnce() -> T) -> Box<T> {
    Box::new(make())
}
#[inline(never)]
pub(super) fn try_boxed<T, E>(make: impl FnOnce() -> Result<T, E>) -> Result<Box<T>, E> {
    make().map(Box::new)
}
#[inline(never)]
pub(super) fn try_optional<T, E>(
    make: impl FnOnce() -> Result<Option<T>, E>,
) -> Result<Option<Box<T>>, E> {
    make().map(|value| value.map(Box::new))
}
