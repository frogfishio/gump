//! Secret-bearing values with fail-closed Debug/Display.

use core::fmt;
use zeroize::Zeroize;

/// Wrapper that never prints plaintext in `Debug` or `Display`.
///
/// Exit evidence for W02: formatting a `Secret` must not leak the inner value.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret<T: Zeroize>(T);

impl<T: Zeroize> Secret<T> {
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &T {
        &self.0
    }

    pub fn expose_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl<T: Zeroize> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_redact() {
        let s = Secret::new(String::from("hunter2"));
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(format!("{s}"), "***");
        assert!(!format!("{s:?}").contains("hunter2"));
    }

    #[test]
    fn expose_still_works_for_authorized_use() {
        let s = Secret::new([1u8, 2, 3, 4]);
        assert_eq!(s.expose(), &[1, 2, 3, 4]);
    }
}
