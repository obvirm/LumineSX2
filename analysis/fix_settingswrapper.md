# Fix: SettingsWrapper C++ → Rust Port

## Perubahan

### 1. File BARU: `rust/common/src/settings_wrapper.rs` (606 lines)

Port lengkap `common/SettingsWrapper.h/.cpp` (367 lines C++) ke Rust:

| Rust Struct | C++ Class | Semantik |
|-------------|-----------|----------|
| `SettingsLoadWrapper<'a>` | `SettingsLoadWrapper` | Reading — `get_*` dari interface |
| `SettingsSaveWrapper<'a>` | `SettingsSaveWrapper` | Writing — `set_*` ke interface, track `modified` |
| `SettingsClearWrapper<'a>` | `SettingsClearWrapper` | Delete — `delete_value` dari interface |

**Method Coverage:**

| Method | Load | Save | Clear |
|--------|------|------|-------|
| `entry_int(section, key, default)` | ✅ | ✅ | ✅ |
| `entry_uint(section, key, default)` | ✅ | ✅ | ✅ |
| `entry_bool(section, key, default)` | ✅ | ✅ | ✅ |
| `entry_float(section, key, default)` | ✅ | ✅ | ✅ |
| `entry_string(section, key, default)` | ✅ | ✅ | ✅ |
| `entry_bitfield(section, key, default)` | ✅ | ✅ | ✅ |
| `entry_bit_bool(section, key, default)` | ✅ | ✅ | ✅ |
| `entry_enum(section, key, names, default)` | ✅ | ✅ | ✅ |
| `is_modified()` | — | ✅ | — |

Juga tersedia macro:
- `settings_section!("SectionName")` — ganti `SettingsWrapSection`
- `settings_entry!(wrapper, var)` — ganti `SettingsWrapEntry` (untuk i32)
- `settings_bitfield!(wrapper, var)` — ganti `SettingsWrapBitfield`
- `settings_bitbool!(wrapper, var)` — ganti `SettingsWrapBitBool`
- `settings_enum!(wrapper, var, names)` — ganti `SettingsWrapEnumEx`
- `settings_parsed_enum!(wrapper, var, parse_fn, name_fn)` — ganti `SettingsWrapParsedEnum`

**Tests:** 8 unit tests (load_roundtrip, load_default, save_then_load, clear_removes, enum_save_and_load, bitfield_roundtrip, macro_load_entry)

### 2. Update: `rust/common/src/lib.rs`

- Added `pub mod settings_wrapper;` (line 106)
- Added `pub use settings_wrapper::*;` (line 213)

### 3. Fix: `rust/common/src/memory_settings_interface.rs`

- Hapus `use std::error::Error;` yang konflik dengan `settings_interface::Error`
- Ganti `Box<dyn Error>` → `Box<dyn std::error::Error>` (fully-qualified path)

## Compile Check

```
cargo check → 26 errors total (semua pre-existing: perf_event_counter, linux_*, dbus, x11)
             0 errors dari settings_wrapper.rs ✅
```

## Residual Risks

- Macro `settings_entry!` hanya support tipe integer (`i32`). Untuk bool/float/string, user panggil method langsung (`w.entry_bool(...)`, `w.entry_float(...)`, `w.entry_string(...)`).
- C++ `SettingsWrapParsedEnum` diport sebagai `settings_parsed_enum!` macro — butuh closure untuk parse/name functions.
- `_shim_settings.cpp` stubs masih ada untuk C++ caller — tidak dihapus karena masih dipakai oleh C++ code yang link ke Rust common. Ini bisa dibersihkan nanti.
