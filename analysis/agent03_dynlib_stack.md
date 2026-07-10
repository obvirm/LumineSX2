# Agent 03: DynamicLibrary + StackWalker — C++ vs Rust Analysis

## DynamicLibrary C++ vs Rust

### Method-by-Method Coverage

| # | C++ DynamicLibrary Method | Rust `dynamic_library.rs` | Status |
|---|--------------------------|---------------------------|--------|
| 1 | `DynamicLibrary()` (default) | `DynamicLibrary::new()` | ✅ |
| 2 | `DynamicLibrary(const char* filename)` | `DynamicLibrary::load(filename)` | ✅ (Result-based, bukan constructor) |
| 3 | `DynamicLibrary(DynamicLibrary&& move)` | Ownership via `Option<Library>` drop | ✅ |
| 4 | `~DynamicLibrary()` | `Drop::drop()` → `self.close()` | ✅ |
| 5 | `IsOpen()` | `is_open()` | ✅ |
| 6 | `Open(const char*, Error*)` | `load(filename)` | ✅ (tidak ada Error parameter — pakai Result) |
| 7 | `Close()` | `close()` | ✅ |
| 8 | `GetSymbolAddress(const char*)` | `get_symbol_address(name)` | ✅ |
| 9 | `GetUnprefixedFilename(filename)` | `add_lib_suffix(filename)` | ✅ |
| 10 | `GetVersionedFilename(libname, major, minor)` | `versioned_filename(libname, major, minor)` | ✅ |
| 11 | `Adopt(void* handle)` | ❌ **MISSING** | ❌ |
| 12 | `GetHandle()` | ❌ **MISSING** | ❌ |
| 13 | `GetSymbol<T>(name, T* ptr)` template | N/A (generics di Rust via caller) | ✅ (tidak perlu template) |
| 14 | Move assignment `operator=(&&)` | Default via `Option` drop | ✅ |

### Misiing: `Adopt()` dan `GetHandle()`

**Dimana dipakai di C++:**
- `common/CrashHandler.cpp:163` — `s_dbghelp_module.Adopt(mod)` — mengadopsi HMODULE dari `LoadDbgHelpLibrary()`
- `common/CrashHandler.cpp:125` — `s_dbghelp_module.GetHandle()` — mengambil handle untuk `MiniDumpWriteDump`

**Severity:** Medium. Dipakai CrashHandler untuk pre-load `dbghelp.dll` sebelum crash, dan untuk minidump writing.

**Fix:** Tambah FFI: `pcsx2_dynlib_adopt_handle(lib, handle)` dan `pcsx2_dynlib_get_handle(lib) → *mut c_void`.

### FFI Surface Coverage

| Rust FFI | Ada? | Digunakan? |
|----------|------|-----------|
| `pcsx2_dynlib_load(name) → *mut DynamicLibrary` | ✅ | Tidak (C++ pake C++ class langsung) |
| `pcsx2_dynlib_get_symbol(lib, name) → *mut c_void` | ✅ | Tidak |
| `pcsx2_dynlib_destroy(lib)` | ✅ | Tidak |
| `pcsx2_dynlib_adopt_handle(lib, handle)` | ❌ | Perlu ditambah |
| `pcsx2_dynlib_get_handle(lib) → *mut c_void` | ❌ | Perlu ditambah |

### C++ Shim (`_shim_dynamiclibrary.cpp`)

Ada file shim tapi **NON-FUNCTIONAL**:
- `Open()` → return `false` (gagal selalu)
- `GetSymbolAddress()` → return `nullptr`
- `GetVersionedFilename()` → tidak tambah suffix platform
- `GetUnprefixedFilename()` → tidak tambah `.dll`/`.so`
- Hanya `Adopt()` dan `Close()` yang jalan (null assignment)

**Status: ⚠️ Shim tidak nyambung ke Rust FFI**. Perlu di-rewrite untuk panggil `pcsx2_dynlib_load` dll.

### Link-time Resolution

- `DynamicLibrary.cpp` masih di `common/CMakeLists.txt` baris 115 ✅ (original C++ dipakai)
- `_shim_dynamiclibrary.cpp` dipakai HANYA jika DynamicLibrary.cpp di-exclude
- Rust `dynamic_library.rs` kompilasi sendiri, FFI tidak dipanggil dari C++

**Kesimpulan:** DynamicLibrary Rust ✅ mandiri, tapi belum diintegrasi ke C++ caller chain.

---

## StackWalker C++ vs Rust

### Method-by-Method Coverage

| # | C++ StackWalker Method | Rust `stack_walker.rs` | Status |
|---|----------------------|------------------------|--------|
| 1 | `StackWalker(options, symPath, processId, process)` constructor | `capture_stack_trace(max_frames)` | ✅ beda API |
| 2 | `StackWalker(processId, process)` constructor | N/A | ✅ (tercover oleh default) |
| 3 | `~StackWalker()` destructor | Automatic (Vec drop) | ✅ |
| 4 | `StackWalker::LoadDbgHelpLibrary()` | ❌ NOT NEEDED (`backtrace` internal) | ✅ |
| 5 | `StackWalker::LoadModules()` | ❌ NOT NEEDED (`backtrace` internal) | ✅ |
| 6 | `StackWalker::ShowCallstack(hThread, context, readMemFn, userData)` | `pcsx2_stack_capture(max, out, count)` | ⚠️ **Partial** |
| 7 | `StackWalker::ShowObject(pObject)` | ❌ **MISSING** | ❌ |
| 8 | `StackWalker::myReadProcMem()` static | ❌ NOT NEEDED (`backtrace` internal) | ✅ |
| 9 | Virtual `OnOutput(szText)` | ❌ NOT NEEDED (struct return) | ✅ |
| 10 | Virtual `OnSymInit(...)` | ❌ NOT NEEDED (struct return) | ✅ |
| 11 | Virtual `OnLoadModule(...)` | ❌ NOT NEEDED (struct return) | ✅ |
| 12 | Virtual `OnCallstackEntry(...)` | ❌ NOT NEEDED (struct return) | ✅ |
| 13 | Virtual `OnDbgHelpErr(...)` | ❌ NOT NEEDED (error via bool return) | ✅ |

### Critical Gap: ShowCallstack dengan CONTEXT

**C++:**
```cpp
BOOL ShowCallstack(HANDLE hThread, const CONTEXT* context, ...);
```
Ini bisa capture stack dari:
- Thread lain (`hThread`)
- Dengan `CONTEXT` dari exception handler (`exi->ContextRecord`)
- Custom memory reader (untuk remote process)

**Rust:**
```rust
pub fn capture_stack_trace(max_frames: usize) -> Vec<StackFrame>
```
Hanya capture stack **dari thread sendiri**, tidak bisa:
- ❌ Accept CONTEXT from exception handler
- ❌ Walk stack of another thread
- ❌ Custom memory reader

**Dimana dipakai:**
- `CrashHandler.cpp:139` — `sw.ShowCallstack(GetCurrentThread(), exi ? exi->ContextRecord : nullptr)`
  - `exi->ContextRecord` adalah `PEXCEPTION_POINTERS::ContextRecord` — memberikan konteks CPU saat crash
  - Ini KRITIKAL untuk crash reporting: tanpa CONTEXT, stack trace setelah crash tidak akan akurat

### Gap: StackFrame fields

| C++ CallstackEntry field | Rust StackFrame field | Status |
|-------------------------|----------------------|--------|
| `offset` (DWORD64) | `address: u64` | ✅ |
| `name` / `undName` / `undFullName` | `symbol: String` | ✅ (merged) |
| `lineFileName` | `file: Option<String>` | ✅ |
| `lineNumber` | `line: Option<u32>` | ✅ |
| `offsetFromSmybol` | ❌ **MISSING** | ⚠️ |
| `offsetFromLine` | ❌ **MISSING** | ⚠️ |
| `symType` | ❌ **MISSING** | ⚠️ |
| `moduleName` | ❌ **MISSING** | ⚠️ |
| `baseOfImage` | ❌ **MISSING** | ⚠️ |
| `loadedImageName` | ❌ **MISSING** | ⚠️ |

**Severity:** Low. `backtrace::BacktraceSymbol` hanya provide: `name()`, `filename()`, `lineno()`. Module/base address info tidak available via backtrace crate.

### FFI Surface Coverage

| Rust FFI | Ada? | Digunakan? |
|----------|------|-----------|
| `pcsx2_stack_capture(max_frames, out_frames, out_count) → bool` | ✅ | Tidak ada C++ shim |
| `pcsx2_stack_free(frames, count)` | ✅ | Tidak ada C++ shim |

### C++ Shim

❌ **TIDAK ADA** `_shim_stackwalker.cpp`. StackWalker tidak punya shim sama sekali. C++ original StackWalker.cpp masih full dipakai.

---

## Summary

| Modul | Rust Pure | FFI Exports | C++ Shim (functional) | Terintegrasi |
|-------|-----------|-------------|----------------------|--------------|
| **DynamicLibrary** | ✅ Complete (minus Adopt/GetHandle) | ✅ 3 fungsi | ⚠️ Ada tapi NON-FUNCTIONAL | ❌ Tidak |
| **StackWalker** | ✅ Basic (hanya self-thread, less fields) | ✅ 2 fungsi | ❌ Tidak ada | ❌ Tidak |

### Open Risks

1. **DynamicLibrary::Adopt() + GetHandle()** — hilang di Rust. Dipakai CrashHandler untuk pre-load dbghelp.dll + minidump writing. Sementara CrashHandler.cpp still uses C++ DynamicLibrary.

2. **StackWalker ShowCallstack() tanpa CONTEXT parameter** — Rust `capture_stack_trace()` tidak bisa accept CONTEXT dari exception handler. Untuk crash reporting, ini menghasilkan stack trace yang TIDAK akurat (karena stack mungkin sudah berubah saat crash handler jalan).

3. **StackWalker thread lain** — Rust hanya capture stack thread sendiri. C++ bisa capture thread lain.

4. **Module info hilang** — moduleName, baseOfImage, loadedImageName, symType tidak ada di Rust. `backtrace` crate tidak provide ini.

5. **C++ shim DynamicLibrary non-functional** — kalau DynamicLibrary.cpp di-exclude dari CMake, VKLoader (vulkan-1.dll), GSCapture (FFmpeg), dan GLContextEGL (libEGL) akan gagal load.
