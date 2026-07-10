# Fix: Semaphore FFI Symbol Conflict — Selesai

## Masalah
`semaphore_impl.rs` dan `threading.rs` sama-sama mengekspor 4 FFI symbol IDENTIK:
- `pcsx2_semaphore_create`
- `pcsx2_semaphore_destroy`
- `pcsx2_semaphore_wait`
- `pcsx2_semaphore_post`

Akibatnya `semaphore_impl` di-DISABLE dari lib.rs.

## Fix
1. **`semaphore_impl.rs`** — di-rewrite total (2957 bytes):
   - Hapus struct `Semaphore` + `impl` + semua `#[no_mangle] pub extern "C"` FFI exports
   - Ganti dengan: `pub use crate::threading::Semaphore;` (re-export tanpa FFI)
   - Tambah `pub type KernelSemaphore = Semaphore;` (alias C++ naming)
   - Tests tetap ada, diadaptasi ke API threading::Semaphore

2. **`lib.rs`** — Enable `pub mod semaphore_impl;` + `pub use semaphore_impl::*;`

## Verifikasi
- `cargo check --features pcsx2-core` → **zero errors** ✅
- `cargo build --features pcsx2-core` → **zero errors** ✅ (LumineSX2.exe 71MB)
- Linker berhasil link tanpa LNK2005 duplicate symbol

## Resiko Residual
- Tidak ada. FFI symbols tetap dari `threading.rs` (tempat yang sama seperti sebelum).
- `semaphore_impl.rs` sekarang pure Rust module tanpa FFI — aman di-enable.
