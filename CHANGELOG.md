# Changelog

## [Unreleased]

- `Image::from_bytes` rejects containers shorter than header plus digest instead of underflowing.
- `find` returns 0 for an empty needle rather than panicking inside `slice::windows`.
- `.maxstack` above the supported frame size is clamped to 4096 slots.

## [3.1.4] - 2026-08-27

- Snapshot restore caps frame and slot counts before allocating.
