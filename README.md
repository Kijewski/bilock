# Bilock: a minimal spin-lock based two-handle mutex pair for `no_std` Rust

[![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/Kijewski/bilock/ci.yml?branch=main&style=flat-square&logo=github&logoColor=white "GitHub Workflow Status")](https://github.com/Kijewski/bilock/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/bilock?logo=rust&style=flat-square "Crates.io")](https://crates.io/crates/bilock)
[![docs.rs](https://img.shields.io/docsrs/bilock?logo=docsdotrs&style=flat-square&logoColor=white "docs.rs")](https://docs.rs/bilock/)

[`Bilock::new()`] provides two linked handles that share ownership of the same
guarded value. A lock is held by either a temporary [`Guard`] or an
[`OwnedGuard`], and the underlying value is released once both handles are
dropped.

The library employs [spin loops](hint::spin_loop) to wait for the lock,
so it is intended for short critical sections only.

# Example

```rust
use bilock::Bilock;

let (mut left, mut right) = Bilock::new(42);
let guard = left.lock();
assert_eq!(*guard, 42);
drop(guard);

let mut guard = right.lock();
*guard = 4711;
assert_eq!(*guard, 4711);
```

## License

This project is tri-licensed under <tt>ISC OR MIT OR Apache-2.0</tt>.
Contributions must be licensed under the same terms.
Users may follow any one of these licenses, or all of them.

See the individual license texts at
* <https://spdx.org/licenses/ISC.html>,
* <https://spdx.org/licenses/MIT.html>, and
* <https://spdx.org/licenses/Apache-2.0.html>.
