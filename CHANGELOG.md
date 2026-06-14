# Changelog

## v0.2.0 (2026-06-14)

- Remove streams feature and Entries stream type (#10)
- Remove unused ReadError type and WriteError::UnexpectedEof variant (#10)
- Fix false debug_assert when writing more data than entry size (#10)
- Propagate poll_finish_entry errors in release builds (#10)
- Update rand crate to v0.10 (#10)
- Update for Rust >=1.92 (#10)

## v0.1.2 (2025-09-23)

- Drop dependency on `thiserror` (#8)

## v0.1.1 (2025-09-16)

- Stop building docs on Windows on CI (#6)
- Disable logging (#5)

## v0.1.0 (2025-09-16)

Initial release.
