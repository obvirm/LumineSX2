# Agent B: String + Path + File — C++ vs Rust Analysis

## File Set

| C++ File | Lines | Rust File | Lines | Coverage |
|----------|-------|-----------|-------|----------|
| `StringUtil.cpp` | 544 | `string_util.rs` | 805 | ⚠️ Partial |
| `StringUtil.h` | 342 | (merged) | | |
| `SmallString.cpp` | 844 | `small_string.rs` | 294 | ⚠️ Alias-only |
| `SmallString.h` | 425 | (merged) | | |
| `Path.h` | 87 | `path.rs` | 1071 | ✅ Complete |
| `FileSystem.cpp` | 2727 | `file_system.rs` | 991 | ✅ Complete |
| `FileSystem.h` | 219 | (merged) | | |

---

## 1. StringUtil — Function-by-Function

### C++ External Dependencies
- `fmt/format.h` — `fmt::format`, `fmt::format_to` (U128 formatting)
- `fast_float/fast_float.h` — float `from_chars`
- `RedtapeWindows.h` — `MultiByteToWideChar`, `WideCharToMultiByte` (Win32)

### Rust Coverage

| # | C++ Function | Rust Equivalent | Status |
|---|-------------|-----------------|--------|
| 1 | `StdStringFromFormat` | `format(format_args!(...))` | ✅ |
| 2 | `StdStringFromFormatV` | `format()` | ✅ |
| 3 | `WildcardMatch` | `wildcard_match()` / `WildcardMatch()` | ✅ |
| 4 | `Strlcpy(char*, const char*, size)` | `strlcpy()` | ✅ |
| 5 | `Strlcpy(char*, string_view, size)` | `strlcpy()` (unified) | ✅ |
| 6 | `Strcasecmp` (inline) | `strcasecmp()` | ✅ |
| 7 | `Strncasecmp` (inline) | `strncasecmp()` | ✅ |
| 8 | `FromChars<T>(str, base=10)` (int) | `from_chars::<T>()` | ✅ |
| 9 | `FromChars<T>(str, base, endptr)` (int) | `parse_with_rest()` | ✅ |
| 10 | `FromChars<T>(str)` (float) | `from_chars::<f64>()` | ✅ |
| 11 | `FromChars<T>(str, endptr)` (float) | `parse_with_rest()` | ✅ |
| 12 | `FromChars<bool>(str)` | `parse_bool()` / `BoolArg` | ✅ |
| 13 | `ToChars<T>(value, base=10)` (int) | `to_chars(value)` | ✅ |
| 14 | `ToChars<T>(value)` (float) | `to_chars(value)` | ✅ |
| 15 | `ToChars<bool>(value)` | `to_chars(value)` | ✅ |
| 16 | `DecodeHex(str)` | ❌ **MISSING** | ❌ |
| 17 | `EncodeHex(data, length)` | ❌ **MISSING** | ❌ |
| 18 | `toLower(str_view)` | ❌ **MISSING** | ❌ |
| 19 | `toUpper(str_view)` | ❌ **MISSING** | ❌ |
| 20 | `compareNoCase(str1, str2)` | ❌ **MISSING** (stub via strcasecmp) | ❌ |
| 21 | `splitOnNewLine(str)` | ❌ **MISSING** | ❌ |
| 22 | `StripWhitespace(str_view)` | ❌ **MISSING** | ❌ |
| 23 | `StripWhitespace(string*)` | ❌ **MISSING** | ❌ |
| 24 | `SplitString(str, delim, skip_empty)` | ❌ **MISSING** | ❌ |
| 25 | `ReplaceAll(subject, search, replacement)` (2 ovlds) | ❌ **MISSING** | ❌ |
| 26 | `ParseAssignmentString(str, key, value)` | ❌ **MISSING** | ❌ |
| 27 | `AppendUTF16CharacterToUTF8(string, u16)` | ❌ **MISSING** | ❌ |
| 28 | `EncodeAndAppendUTF8(string, char32_t)` | ❌ **MISSING** | ❌ |
| 29 | `DecodeUTF8(bytes, len, ch)` (3 ovlds) | ❌ **MISSING** | ❌ |
| 30 | `Ellipsise(str, max_len, "...")` (2 ovlds) | ❌ **MISSING** | ❌ |
| 31 | `UTF8StringToWideString(str)` (Win32) | ❌ **MISSING** (Win32-only) | 🔲 |
| 32 | `WideStringToUTF8String(wstr)` (Win32) | ❌ **MISSING** (Win32-only) | 🔲 |
| 33 | `U128ToString(u128)` | ❌ **MISSING** | ❌ |
| 34 | `AppendU128ToString(u128, string)` | ❌ **MISSING** | ❌ |
| 35 | `StrideMemCpy` (inline) | ❌ **MISSING** (inline) | 🔲 |
| 36 | `StrideMemCmp` (inline) | ❌ **MISSING** (inline) | 🔲 |
| 37 | `ContainsSubString` (template) | Rust `str::contains` | ✅ |

**Severity:**
- 🔴 **SEVERAL MISSING**: `toLower`, `toUpper`, `ReplaceAll`, `StripWhitespace`, `SplitString`, `Ellipsise`, `DecodeHex`, `EncodeHex` — function DIPAKAI DI MANA-MANA
- 🔴 **U128 format missing**: dipakai di logging/debug core
- 🟡 **UTF8 encode/decode missing**: dipakai di settings loading
- 🟢 Platform-specific (UTF16/32 Win32): low priority

### Dimana Rust Functions Berada (selain string_util.rs)
- `toLower`/`toUpper` → `str.to_lowercase()`/`to_uppercase()` built-in (tapi tidak di-export via FFI)
- `splitOnNewLine` → `.split('\n').collect()`
- `StripWhitespace` → `.trim()`
- `ReplaceAll` → `.replace()`
- `U128ToString` → `fmt::format()` di call site

**Tapi FFI export TIDAK ADA** untuk fungsi-fungsi ini! C++ side tidak bisa akses.

---

## 2. SmallString — Analysis

### C++ Class Design
```
SmallStackString<256> (SmallString) — stack[256] + heap fallback
SmallStackString<64>  (TinyString) — stack[64]  + heap fallback
```

~60 methods: constructors, assign, clear, append, prepend, insert, sprintf, format, equals, iequals, compare, icompare, starts_with, ends_with, find, rfind, count, erase, reserve, resize, shrink_to_fit, const char* operator

### Rust Implementation
```rust
pub type SmallStringBase = String;
pub type SmallString = String;
pub type TinyString = String;
```

**⚠️ CATASTROPHIC SIMPLIFICATION:**
- **SSO hilang total** — SmallString (256-byte stack) vs String (always heap). Setiap `TinyString` alokasi heap sekarang!
- **Tidak ada method wrapper** — `String` punya method sendiri, tapi API berbeda (`push_str()` vs `append()`, `insert_str()` vs `insert()`)
- **C++ fmt::format integration hilang** — `MAKE_FORMATTER(SmallString)` agar bisa `fmt::format("{}", str)` tidak ada di Rust
- **append_sprintf / prepend_sprintf** — tidak ada (pake `write!`/`format!`)
- **append_format / prepend_format** — tidak ada
- **starts_with / ends_with** — via `str::starts_with` ✅ (tapi casing-sensitive beda)
- **find / rfind** — via `str::find`/`rfind` ✅
- **reserve / resize** — via `String::reserve`/`resize` ✅

### Fungsi C++ yang HILANG di Rust

| Method | Rust Alternatif | Status |
|--------|----------------|--------|
| `append_sprintf` | Tidak ada | ❌ |
| `prepend_sprintf` | Tidak ada | ❌ |
| `append_format` | `write!` macro | ⚠️ |
| `prepend_format` | Tidak ada | ❌ |
| `prepend_vsprintf` | Tidak ada | ❌ |
| `insert(offset, str)` | `String::insert_str()` | ✅ |
| `append_hex` | Tidak ada | ❌ |
| `vformat` | `write!` | ⚠️ |
| `sprintf` | `format!` | ⚠️ |
| `iequals` (case-insensitive) | Ada via `iequals()` fn ✅ | ✅ |
| `icompare` | Tidak ada | ❌ |
| `count(ch)` | Ada via `count()` fn | ✅ |
| `starts_with(str, case_sensitive)` | `starts_with` (only case-sensitive) | ⚠️ |
| `ends_with(str, case_sensitive)` | `ends_with` (only case-sensitive) | ⚠️ |
| C++ `operator const char*` | `as_str()` | ⚠️ |
| C++ `operator std::string_view` | `as_str()` | ⚠️ |

**Severity: HIGH** — SmallString dipakai DI MANA-MANA di PCSX2 (settings, paths, config, logging).
`TinyString` dan `SmallString` adalah tipe string UTAMA di PCSX2 C++.

---

## 3. Path — Function-by-Function ✅

**✅ ALL 23 C++ functions ported to Rust**
**✅ 10 FFI exports**
**✅ Proper `std::path::Path` delegation**
**✅ Cross-platform (Windows/Unix conditional compilation)**
**✅ URL encode/decode via RFC 3986**
**✅ Extensive test suite**

| Gap | Detail | Severity |
|-----|--------|----------|
| C++ `BuildRelativePath` | Rust `build_relative_path` — path separator semantics beda tipis | 🟢 Low (tidak critical) |

**Path: SATU-SATUNYA module yang full coverage.**

---

## 4. FileSystem — Function-by-Function

### Rust Coverage ✅
- `file_exists`, `directory_exists`, `directory_is_empty` — ✅
- `stat_file`, `path_file_size`, `file_timestamp` — ✅ (tidak ada `stat(FILE*)` overload)
- `delete_file_path`, `rename_path` — ✅
- `read_binary_file`, `read_file_to_string`, `write_binary_file`, `write_string_to_file` — ✅
- `Mmap` (dengan `memmap2` crate) — ✅
- `create_directory`, `create_directory_path`, `ensure_directory_exists` — ✅
- `delete_directory`, `recursive_delete_directory` — ✅
- `copy_file_path` — ✅
- `find_files` (glob search dengan `*`/`?`) — ✅
- `get_working_directory`, `set_working_directory` — ✅
- `get_program_path` — ✅
- `get_root_directory_list` — ✅
- symlink helpers (Unix) — ✅

### Missing

| # | C++ Function | Alasan | Severitas |
|---|-------------|--------|-----------|
| 1 | `OpenCFile(const char*, mode, Error*)` | C `FILE*` — Rust pake `File` | 🟢 Low |
| 2 | `OpenManagedCFile(filename, mode, Error*)` | `unique_ptr<FILE>` — Rust `File` otomatis RAII | 🟢 Low |
| 3 | `OpenSharedCFile(filename, mode, share_mode, Error*)` | Win32 `_fsopen` — Rust `File` tidak support sharing flags | 🟡 Medium |
| 4 | `StatFile(FILE*, ...)` | Same — `FILE*` | 🟢 Low |
| 5 | `FSeek64/FTell64/FSize64` | Rust `File::seek/stream_len` | 🟢 Low |
| 6 | `OpenFDFile` (POSIX fd) | Rust `File::open` | 🟢 Low |
| 7 | `MapBinaryFileForRead(FILE*)` | Rust `Mmap::open` sudah handle | ✅ |
| 8 | `UnmapFile(span)` | Rust `Mmap` drop otomatis | ✅ |
| 9 | `SetPathCompression(path, bool)` | Win32 NTFS-specific (FSCTL_SET_COMPRESSION) | 🟢 Low |
| 10 | `GetPackagePath()` | AppImage-specific (Linux) | 🟢 Low |
| 11 | `GetWin32Path(string)` | Win32 long-path prefix | 🟡 Medium |
| 12 | `POSIXLock` | POSIX file locking | 🟢 Low |
| 13 | `CreateSymLink` | Rust `std::os::unix::fs::symlink` ✅ | ✅ |
| 14 | `IsSymbolicLink` | Rust `fs::symlink_metadata` ✅ | ✅ |
| 15 | `DeleteSymbolicLink` | Rust `fs::remove_file` ✅ | ✅ |

**External Rust crates:**
- `bitflags` — bitflag enums (FileAttributes, FindFlags)
- `memmap2` — memory-mapped files

**External C++ libs:**
- `RedtapeWindows.h` → Windows API (CreateFile, GetFileInformationByHandle, etc.)
- `<io.h>` → `_access`, `_chsize_s`
- `<pathcch.h>` → `PathCchCombine`, `PathCchCanonicalize`
- `<winioctl.h>` → `FSCTL_SET_COMPRESSION`
- `<shlobj.h>` → `SHGetKnownFolderPath`
- `<dirent.h>` → `opendir`, `readdir` (POSIX)
- `<sys/mman.h>` → `mmap`, `munmap` (POSIX)

---

## Ringkasan Severitas

### 🔴 KRITIS — Harus segera diperbaiki
| Module | Missing |
|--------|---------|
| **StringUtil** | `toLower`, `toUpper`, `ReplaceAll`, `StripWhitespace`, `SplitString`, `Ellipsise`, `DecodeHex`, `EncodeHex` — semua DIPAKAI DI MANA-MANA di C++ core |
| **SmallString** | SSO hilang. Setiap `TinyString`/`SmallString` allocation heap. `append_sprintf`, `prepend_*`, `vformat` — dipakai di setting load/path building. |

### 🟡 SEDANG
| Module | Missing |
|--------|---------|
| **StringUtil** | UTF8 encode/decode — dipakai di settings parser. `U128ToString` — dipakai debug logging. |
| **SmallString** | `starts_with/ends_with` case-insensitive — dipakai di path matching. |
| **FileSystem** | `OpenSharedCFile` — dipakai memory card file access. `GetWin32Path` — dipakai untuk long path >260 chars. |

### 🟢 RENDAH
| Module | Missing |
|--------|---------|
| FileSystem | `SetPathCompression`, `POSIXLock`, `GetPackagePath` — edge cases |
| StringUtil | Platform-specific UTF16 conversion, inline helpers |
| SmallString | `icompare`, `prepend` overloads — bisa pakai `String` method langsung |

### ✅ SUDAH OK
| Module | Verdict |
|--------|---------|
| **Path** | ✅ 100% complete. 20+ functions. 10 FFI exports. |
| **FileSystem** | ✅ Core functions semua ada. Missing functions minor/platform-specific. |

---

## External Crate Dependencies

| Rust Crate | Command | Untuk |
|-----------|---------|-------|
| **`bitflags`** | `cargo add bitflags` | FileAttributes, FindFlags bitflags ✅ (udah ada) |
| **`memmap2`** | `cargo add memmap2` | Memory-mapped file I/O ✅ (udah ada) |
| **`libc`** | `cargo add libc` | `FILE*` fread, malloc, free ✅ (udah ada) |

## Kesimpulan

**StringUtil dan SmallString perlu perbaikan besar.** Path dan FileSystem sudah OK.
- StringUtil.hilang ~20 fungsi umum yang dipakai di seluruh codebase
- SmallString cuma alias ke String — SSO requirement tidak terpenuhi

---

*Awalnya saya kira coverage 100% karena file-path analysis saja, tapi setelah baca implementasi detail, ternyata banyak yang missing.*
