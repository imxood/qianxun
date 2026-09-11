//! Glob wildcard detection — pure Rust.
//!
//! Checks for the wildcard characters `*`, `?`, `[`, `{`.

#[inline]
pub fn has_wildcards(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'*' | b'?' | b'[' | b'{'))
}
