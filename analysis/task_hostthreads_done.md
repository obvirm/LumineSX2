# Task Report: Enable + Fix windows_host_sys + windows_threads

## Summary
Rewrote `windows_host_sys.rs` and `windows_threads.rs` to compile cleanly. Enabled both modules in `lib.rs`.

## Root Cause
`windows_host_sys.rs` had duplicate functions/FFI exports that conflicted with `host_sys.rs` and `host_sys_ffi.rs`. `windows_threads.rs` had missing `BOOL`/`FILETIME` imports from `windows-sys`.

## Changes
1. **windows_host_sys.rs** — Stripped to only unique functions (mem_protect, shared memory, abort). Removed 6 duplicate functions + 5 duplicate FFI exports.
2. **windows_threads.rs** — Added missing `BOOL`, `FILETIME`, `FALSE`, `INFINITE` imports. Removed local constant definitions.
3. **lib.rs** — Uncommented module declarations + pub use for both modules.

## Build Result
0 errors from our modules. 30 pre-existing errors in unrelated Linux modules (perf_event, dbus, x11).
