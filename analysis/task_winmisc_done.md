# Task: WinMisc + Redtape Rust Port — Done

## Changed Files

| File | Action | LOC | Description |
|------|--------|-----|-------------|
| `E:\project\pcsx2\rust\common\src\windows_misc.rs` | **NEW** | 170 | Port of `common/Windows/WinMisc.cpp` — Common::InhibitScreensaver, SetMousePosition, PlaySoundAsync + FFI exports |
| `E:\project\pcsx2\rust\common\src\redtape.rs` | **NEW** | 191 | Port of `common/RedtapeWilCom.h` — ComInitScope (RAII CoInitialize/Couninitialize guard) + CoTaskMemFree helper |
| `E:\project\pcsx2\rust\common\src\lib.rs` | **MODIFIED** | +2 lines | Added `pub mod windows_misc` and `pub mod redtape` + re-exports |
| `E:\project\pcsx2\rust\common\Cargo.toml` | **MODIFIED** | +1 line | Added `"Win32_System_Com"` feature for windows-sys |

## FFI Surface Exported

### In `windows_misc.rs`:
```
pcsx2_windows_inhibit_screensaver(bool) -> bool     — SetThreadExecutionState
pcsx2_windows_set_mouse_position(i32, i32)           — SetCursorPos
pcsx2_windows_attach_mouse_position_cb() -> bool     — acknowledge raw-input support
pcsx2_windows_detach_mouse_position_cb()             — no-op
pcsx2_windows_play_sound_async(*const c_char) -> bool — PlaySoundW (UTF-8→UTF-16)
```

### In `redtape.rs`:
```
ComInitScope::new()                                   — RAII CoInitializeEx/CoUninitialize
ComInitScope::with_model(u32) — unsafe                — custom concurrency model
pcsx2_redtape_co_task_mem_free(*mut c_void)            — CoTaskMemFree wrapper
```

## What was NOT ported (already exists in Rust)

| WinMisc.cpp function | Rust location |
|----------------------|---------------|
| `GetTickFrequency()` | `timer.rs` → `pcsx2_timer_get_tick_frequency()` |
| `GetCPUTicks()` | `timer.rs` → `pcsx2_timer_get_cpu_ticks()` |
| `GetPhysicalMemory()` | `host_sys_ffi.rs` → `pcsx2_host_physical_memory()` |
| `GetAvailablePhysicalMemory()` | `host_sys_ffi.rs` → `pcsx2_host_available_memory()` |
| `GetOSVersionString()` | `host_sys_ffi.rs` → `pcsx2_host_os_version_string()` |
| `Threading::Sleep(int)` | `threading.rs` → `sleep(u32)` |
| `Threading::SleepUntil(u64)` | `threading.rs` → `sleep_until(u64)` |
| `GetCPUInfo()` | `cpu_features.rs` (x86_64 only) |
| `AbortWithMessage()` | `host_sys_ffi.rs` → `pcsx2_host_abort_with_message()` |

## Validation

- `cargo check` — **0 errors, 0 warnings** from new modules
- All 31 pre-existing errors in other modules (SSE 4.1, perf_event, image, etc.)

## Next Steps

- Update `common/_rust_shim/_shim_extras.cpp` stubs to call the new `pcsx2_windows_*` FFI functions instead of returning false/no-op
- Enable `windows_host_sys.rs` once windows-sys API mismatches are resolved
- Enable `windows_threads.rs` same way
