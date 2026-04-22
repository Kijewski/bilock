# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [unreleased]

### Changed

- Simplified allocation

## [v0.1.1] - 2026-04-22

### Added

- `Bilock` creation shortcuts:
  `fn Bilock::new_locked()`, `fn Bilock::new_unpaired()`, `fn OwnedGuard::new()`
- Reviving paired handles:
  `fn Bilock::revive(&mut OwnedGuard)`, `unsafe fn Bilock::revive_unchecked(&mut OwnedGuard)`
- Ease of use for `trait BilockLike`:
  `impl<T: BilockLike> BilockLike for &T {}`

## [v0.1.0] - 2026-04-21

- First release

[unreleased]: <https://github.com/Kijewski/bilock/compare/v0.1.1...HEAD>
[v0.1.1]: <https://github.com/Kijewski/bilock/releases/tag/v0.1.2>
[v0.1.1]: <https://github.com/Kijewski/bilock/releases/tag/v0.1.1>
[v0.1.0]: <https://github.com/Kijewski/bilock/releases/tag/v0.1.0>
