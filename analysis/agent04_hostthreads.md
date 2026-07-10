# Agent 04: WinHostSys + WinThreads — C++ vs Rust Analysis

## File Dipanggil

| C++ File | Rust File | Rust File (cross-platform) |
|----------|-----------|---------------------------|
| `common/Windows/WinHostSys.cpp` (361 lines) | `windows_host_sys.rs` (200 lines) | `host_sys.rs` (800+ lines) |
| `common/Windows/WinThreads.cpp` (289 lines) | `windows_threads.rs` (391 lines) | `threading.rs` (1400+ lines) |

---

## 1. `WinHostSys.cpp` → Coverage

### `HostSys::` static methods

| C++ Function | Rust File | Rust Function | Status |
|---|---|---|---|
| `HostSys::MemProtect()` | `windows_host_sys.rs` | `mem_protect()` + FFI `pcsx2_host_mem_protect` | ✅ |
| `HostSys::GetFileMappingName()` | **❌ MISSING** | — | ❌ |
| `HostSys::CreateSharedMemory()` | `windows_host_sys.rs` | `create_shared_memory()` | ✅ |
| `HostSys::DestroySharedMemory()` | `windows_host_sys.rs` | `destroy_shared_memory()` | ✅ |
| `HostSys::GetRuntimePageSize()` | `host_sys.rs` | `get_runtime_page_size()` | ✅ |
| `HostSys::GetRuntimeCacheLineSize()` | `host_sys.rs` | `get_runtime_cache_line_size()` | ✅ |
| `HostSys::FlushInstructionCache()` | **❌ MISSING** (ARM64 only) | — | ❌ (low — ARM64) |

### `SharedMemoryMappingArea` class

| C++ Method | Rust Equivalent | Status |
|---|---|---|
| Constructor (base_ptr, size, num_pages) | `SharedMemoryMappingArea { base_ptr, size }` | ⚠️ **STUB** (no num_pages, no placeholder_ranges) |
| Destructor (~SharedMemoryMappingArea) | **❌ MISSING** (no `Drop` impl) | ❌ |
| `Create(size, jit)` | **❌ MISSING** (no `VirtualAlloc2`) | ❌ |
| `FindPlaceholder(offset)` | **❌ MISSING** | ❌ |
| `Map(file_handle, offset, base, size, mode)` | **❌ MISSING** (no `MapViewOfFile3`, `VirtualAlloc2`) | ❌ |
| `Unmap(base, size, is_file)` | **❌ MISSING** (no `UnmapViewOfFile2`, coalesce) | ❌ |

### `PageFaultHandler` namespace

| C++ Function | Rust Equivalent | Status |
|---|---|---|
| `PageFaultHandler::Install()` | **❌ MISSING** | ❌ |
| `PageFaultHandler::InstallSecondaryThread()` | **❌ MISSING** (trivial) | ❌ |
| `PageFaultHandler::ExceptionHandler()` | **❌ MISSING** | ❌ |
| Internal: `ConvertToWinApi()` | (inline di `mem_protect()`) | ✅ |

---

## 2. `WinThreads.cpp` → Coverage

### Threading free functions

| C++ Function | Rust File | Rust Function | Status |
|---|---|---|---|
| `Threading::Timeslice()` | `threading.rs` + `windows_threads.rs` | `timeslice()` | ⚠️ DUPLICATE |
| `Threading::SpinWait()` | `threading.rs` + `windows_threads.rs` | `spin_wait()` | ⚠️ DUPLICATE |
| `Threading::EnableHiresScheduler()` | `threading.rs` + `windows_threads.rs` | `enable_hires_scheduler()` | ⚠️ DUPLICATE |
| `Threading::DisableHiresScheduler()` | `threading.rs` + `windows_threads.rs` | `disable_hires_scheduler()` | ⚠️ DUPLICATE |
| `Threading::GetThreadCpuTime()` | `threading.rs` + `windows_threads.rs` | `get_thread_cpu_time()` | ⚠️ DUPLICATE |
| `Threading::SetNameOfCurrentThread()` | `threading.rs` + `windows_threads.rs` | `set_name_of_current_thread()` | ⚠️ DUPLICATE |
| `Threading::Sleep()` | `threading.rs` + `windows_threads.rs` | `sleep()` | ⚠️ DUPLICATE |
| `Threading::SleepUntil()` | `threading.rs` + `windows_threads.rs` | `sleep_until()` | ⚠️ DUPLICATE |
| `Threading::GetThreadTicksPerSecond()` | `threading.rs` | `get_thread_ticks_per_second()` | ✅ |

### ThreadHandle class

| C++ Method | Rust Equivalent | Status |
|---|---|---|
| `ThreadHandle::GetForCallingThread()` | `threading.rs::ThreadHandle::for_calling_thread()` | ✅ |
| `ThreadHandle::GetCPUTime()` | `threading.rs::ThreadHandle::cpu_time()` | ✅ |
| `ThreadHandle::SetAffinity()` | `threading.rs::ThreadHandle::set_affinity()` | ✅ |
| `ThreadHandle` copy/move/destructor | Rust ownership (Drop) | ✅ |

### Thread class

| C++ Method | Rust Equivalent | Status |
|---|---|---|
| `Thread::Thread(func)` | `threading.rs::Thread::new()` + `start()` | ✅ |
| `Thread::SetStackSize()` | `threading.rs::Thread::set_stack_size()` | ✅ |
| `Thread::Start()` | `threading.rs::Thread::start()` | ✅ |
| `Thread::Detach()` | (drop JoinHandle) | ✅ |
| `Thread::Join()` | `threading.rs::Thread::join()` | ✅ |
| `Thread::ThreadProc()` | std::thread::spawn closure | ✅ |

---

## 3. 🔴 CRITICAL: Duplicate Symbols (Akan ERROR di Windows)

**8 fungsi** didefinisikan di **DUA file berbeda**: `threading.rs` dan `windows_threads.rs`.

| Fungsi | threading.rs | windows_threads.rs | Problem |
|--------|-------------|-------------------|---------|
| `timeslice()` | ✅ (yield_now) | ✅ (SwitchToThread) | ⚠️ DUPLICATE |
| `spin_wait()` | ✅ (spin_loop) | ✅ (spin_loop) | ⚠️ DUPLICATE |
| `enable_hires_scheduler()` | ✅ (timeBeginPeriod via FFI) | ✅ (timeBeginPeriod via windows-sys) | ⚠️ DUPLICATE |
| `disable_hires_scheduler()` | ✅ (timeEndPeriod via FFI) | ✅ (timeEndPeriod via windows-sys) | ⚠️ DUPLICATE |
| `get_thread_cpu_time()` | ✅ (QueryThreadCycleTime FFI) | ✅ (GetThreadTimes via windows-sys) | ⚠️ DUPLICATE + DIFFERENT IMPL |
| `set_name_of_current_thread()` | ✅ (SetThreadDescription FFI) | ✅ (SetThreadDescription via windows-sys) | ⚠️ DUPLICATE |
| `sleep()` | ✅ (yield_now + Duration) | ✅ (yield_now + Duration) | ⚠️ DUPLICATE |
| `sleep_until()` | ✅ (Duration::from_micros) | ✅ (CreateWaitableTimer) | ⚠️ DUPLICATE + DIFFERENT IMPL |

### Kenapa cuma compile di Linux?
- Di Linux: `windows_threads.rs` compile sebagai module **empty** (karena `#[cfg(windows)]` gate di `mod imp` dan `pub use imp::*`)
- Di Windows: Kedua module akan export symbol yang sama → **`error[E0428]`**

### Yang paling bahaya: `get_thread_cpu_time()` dan `sleep_until()` punya implementation BERBEDA di kedua file
- `threading.rs` pakai `QueryThreadCycleTime` (CPU cycles)
- `windows_threads.rs` pakai `GetThreadTimes` (FILETIME 100ns ticks)

Ini nilai return-nya BEDA — bisa cause bug silent!

---

## 4. Summary

| Kategori | Count | Detail |
|----------|-------|--------|
| ✅ COVERED | 9 | mem_protect, create/destroy_shared_memory, page_size, cache_line, ThreadHandle, Thread |
| ❌ MISSING (WinHostSys) | 6 | GetFileMappingName, FlushInstructionCache, PageFaultHandler, SharedMemoryMappingArea stub |
| ⚠️ DUPLICATE (akan error) | 8 | timeslice, spin_wait, enable/disable_hires, get_thread_cpu_time, set_name, sleep, sleep_until |
| ⚠️ DUPLICATE + DIFFERENT VAL | 2 | `get_thread_cpu_time` (cycles vs 100ns), `sleep_until` (micros vs waitable timer) |

### Rekomendasi Design

**Opsi A:** Hapus fungsi overlap dari `windows_threads.rs`, sisakan hanya yang benar-benar BEDA implementasi (sleep_until pakai waitable timer). Gunakan `threading.rs` sebagai single source of truth.

**Opsi B:** Balikkan: `threading.rs` hanya berisi fallback portable. `windows_threads.rs` override semua dengan Win32 langsung. `lib.rs` pake cfg:

```rust
#[cfg(windows)]
pub use windows_threads::*;  // override
#[cfg(not(windows))]
pub use threading::*;        // fallback
```

Saya rekomendasi **Opsi A** — hapus duplikasi dari `windows_threads.rs`, karena `threading.rs` sudah punya implementasi Windows via `#[cfg(windows)]` di dalamnya. `windows_threads.rs` hanya perlu fungsi yg bener-bener unik (misal `sleep_until` yang butuh waitable timer precision).
