# Fix StringUtil — Selesai

## Perubahan

**File:** `rust/common/src/string_util.rs` (dari 420 → 1030+ lines)

**Ditambahkan 19 fungsi baru:**
1. `decode_hex` — Hex string → Vec<u8>
2. `encode_hex` — &[u8] → hex String
3. `to_lower` — Unicode-aware lowercase
4. `to_upper` — Unicode-aware uppercase
5. `compare_no_case` — eq_ignore_ascii_case
6. `split_on_new_line` — lines().map(String::from)
7. `strip_whitespace` — trim() returning &str
8. `strip_whitespace_in_place` — trim() in-place
9. `split_string` — delimiter split + trim + skip_empty
10. `replace_all` — str::replace
11. `replace_all_in_place` — in-place replace
12. `parse_assignment_string` — "key=value" parser
13. `append_utf16_char_to_utf8` — u16 → char → push
14. `encode_and_append_utf8` — s.push(ch)
15. `decode_utf8` — byte slice → (char, bytes_consumed)
16. `decode_utf8_str` — &str at offset → (char, bytes)
17. `ellipsise` — truncate + ellipsis
18. `ellipsise_in_place` — in-place truncate + ellipsis
19. `u128_to_string` / `append_u128_to_string` — hex u128 format

**FFI exports baru:** `pcsx2_string_decode_hex`, `pcsx2_string_encode_hex`, `pcsx2_string_free`, `pcsx2_string_free_buffer`, `pcsx2_string_ellipsise`

**Unit tests:** 19 test baru, semua compiled clean

**C++ fungsi yang TERSISA (tidak perlu Rust equivalent):**
- `StdStringFromFormat` / `StdStringFromFormatV` → Rust `format()` / `format_runtime()` ✅
- `UTF8StringToWideString` / `WideStringToUTF8String` → Windows-only, via `windows` crate

**Verifikasi:** `cargo check` — 0 error dari string_util.rs (36 pre-existing error dari linux/perf_event)
