// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_common.h
//
// Shared declarations for the Rust-shim implementation of PCSX2's
// `common/` C++ classes and free functions.
//
// Each `_shim_*.cpp` file in this directory implements a small set of
// C++ classes / free functions by either:
//
//   1. Forwarding to a `pcsx2_*` symbol exported from
//      `rust/common/pcsx2_common_rs` (the Rust reimplementation), or
//   2. Returning a safe default / no-op when no FFI equivalent exists.
//
// The shim exists so we can build `pcsx2-gsrunner.exe` with
// `EXCLUDE_CPP_COMMON=ON`, i.e. without compiling any of the original
// C++ `common/*.cpp` source files. The Rust staticlib
// (`libpcsx2_common_rs.lib`) provides every FFI symbol the C++ core
// needs; this shim adapts the C++ class layout to the C-ABI surface
// exposed by the Rust side.
//
// IMPORTANT: We deliberately do NOT `#include "pcsx2_common_rs.h"` from
// here. The cbindgen-generated header has several declarations that
// are not portable across the shim's translation units (duplicate
// `extern "C"` overloads, missing pthread/mach headers on Windows,
// etc.). Instead each shim cpp file declares only the specific FFI
// symbols it forwards to.
//
// The FFI symbols below are declared with `void*` for the opaque Rust
// handles (`Error*`, `MD5Digest*`, etc.) so that the shim cpp files
// can pass pointers to C++ class instances directly. The two pointer
// types are interchangeable at the ABI level (both are 8-byte
// "pointer to opaque object" values) but the C++ type system doesn't
// allow the implicit conversion otherwise.

#pragma once

#include "common/Pcsx2Defs.h"
#include "common/Pcsx2Types.h"

#include <cstdarg>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

// ---------------------------------------------------------------------------
// LogLevel / ConsoleColor enums.
//
// The Rust FFI uses packed `u32` enums; the C++ side uses the
// `LOGLEVEL` / `ConsoleColors` enums declared in `common/Console.h`.
// They share the same numeric values, so the shim uses static_cast
// at the boundary.
// ---------------------------------------------------------------------------

namespace pcsx2
{
	enum class LogLevel : uint32_t
	{
		LogLevel_None = 0,
		LogLevel_Error = 1,
		LogLevel_Warning = 2,
		LogLevel_Info = 3,
		LogLevel_Dev = 4,
		LogLevel_Debug = 5,
		LogLevel_Trace = 6,
	};

	enum class ConsoleColor : uint32_t
	{
		ConsoleColor_Default = 0,
	};
} // namespace pcsx2

// ---------------------------------------------------------------------------
// Direct C-ABI declarations of every `pcsx2_*` symbol the shim forwards
// to. Each entry matches the corresponding `extern "C"` declaration in
// `rust/common/pcsx2_common_rs.h`. The opaque handle pointers are
// declared as `void*` (rather than `::pcsx2::Error*` etc.) so that
// the shim cpp files can pass pointers to C++ class instances without
// needing a cast.
// ---------------------------------------------------------------------------

extern "C" {

// ---- Assertions ------------------------------------------------------------
void pcsx2_on_assert_fail(const char* file, int line, const char* func, const char* msg);

// ---- Error (opaque handle = C++ Error* or pcsx2::Error*, treated as void*) -
void pcsx2_error_clear(void* err);
void pcsx2_error_destroy(void* err);
void* pcsx2_error_create();
unsigned int pcsx2_error_get_message(void* err, char* out, unsigned int out_len);
bool pcsx2_error_is_valid(void* err);
void pcsx2_error_set_errno(void* err, int code);
void pcsx2_error_set_hresult(void* err, const char* description, int hr);
void pcsx2_error_set_hresult_inst(void* err, int code);
void pcsx2_error_set_hresult_prefix(void* err, const char* prefix, int code);
void pcsx2_error_set_string(void* err, const char* msg);
void pcsx2_error_set_win32_code(void* err, unsigned int code);
void pcsx2_error_set_win32_inst(void* err, unsigned int code);
void pcsx2_error_set_win32_prefix(void* err, const char* prefix, unsigned int code);

// ---- FileSystem ------------------------------------------------------------
bool pcsx2_create_directory(const char* path, bool recursive);
bool pcsx2_delete_file(const char* path);
bool pcsx2_directory_exists(const char* path);
bool pcsx2_file_exists(const char* path);
bool pcsx2_filesystem_directory_is_empty(const char* path);
uint64_t pcsx2_filesystem_read_file_with_progress(void* fp, void* buffer, uint64_t size, void* progress, void* err, uint64_t base_offset);
uint64_t pcsx2_filesystem_read_file_with_partial_progress(void* fp, void* buffer, uint64_t size, void* progress, int min_step, int max_step, void* err, uint64_t base_offset);
uint32_t pcsx2_get_program_path(char* out, uint32_t out_len);
uint32_t pcsx2_get_working_directory(char* out, uint32_t out_len);
bool pcsx2_read_binary_file(const char* path, uint8_t** out, uintptr_t* out_len);
bool pcsx2_rename_file(const char* old_path, const char* new_path);
bool pcsx2_set_working_directory(const char* path);
bool pcsx2_write_binary_file(const char* path, const uint8_t* data, uintptr_t len);

// ---- Path ------------------------------------------------------------------
char* pcsx2_path_append_directory(const char* base, const char* dir);
bool pcsx2_path_change_file_name(char* path, const char* new_file_name);
uint32_t pcsx2_path_combine(char* out, uint32_t out_len, const char* base, const char* next);
uint32_t pcsx2_path_get_directory(char* out, uint32_t out_len, const char* path);
uint32_t pcsx2_path_get_extension(char* out, uint32_t out_len, const char* path);
uint32_t pcsx2_path_get_file_name(char* out, uint32_t out_len, const char* path);
uint32_t pcsx2_path_get_file_title(char* out, uint32_t out_len, const char* path);
uint32_t pcsx2_path_is_absolute(const char* path);
char* pcsx2_path_join_native_path(const char* const* parts, uintptr_t count);
uint32_t pcsx2_path_to_native_path(char* out, uint32_t out_len, const char* path);

// ---- Log -------------------------------------------------------------------
::pcsx2::LogLevel pcsx2_log_get_level();
void pcsx2_log_set_level(::pcsx2::LogLevel level);
void pcsx2_log_set_host_callback(::pcsx2::LogLevel level, void (*callback)(::pcsx2::LogLevel, ::pcsx2::ConsoleColor, const char*, uintptr_t));
void pcsx2_log_write(::pcsx2::LogLevel level, ::pcsx2::ConsoleColor color, const char* msg, uintptr_t len);

// ---- Crash handler ---------------------------------------------------------
bool pcsx2_crash_handler_install();
void pcsx2_crash_handler_set_write_directory(const char* dir);
void pcsx2_crash_handler_write_dump();

// ---- Host ------------------------------------------------------------------
uint64_t pcsx2_host_physical_memory();
uint64_t pcsx2_host_available_memory();
uint64_t pcsx2_host_cpu_ticks();
uint64_t pcsx2_host_page_size();

// ---- Timer -----------------------------------------------------------------
uint64_t pcsx2_timer_get_cpu_ticks();
uint64_t pcsx2_timer_get_tick_frequency();
uint64_t pcsx2_timer_get_ticks();
double pcsx2_timer_get_ticks_as_seconds(uint64_t ticks);

// ---- HTTP ------------------------------------------------------------------
bool pcsx2_http_download(const char* url, const char* dest_path);

// ---- MD5 -------------------------------------------------------------------
void pcsx2_md5_destroy(void* ctx);
void pcsx2_md5_final(void* ctx, uint8_t* out);
void pcsx2_md5_hash(const uint8_t* data, uint32_t len, uint8_t* out);
void* pcsx2_md5_new();
void pcsx2_md5_update(void* ctx, const uint8_t* data, uint32_t len);

// ---- Perf ------------------------------------------------------------------
void pcsx2_perf_group_register_key(void* group, const void* key, uint64_t name_hash, const char* name, uint64_t display_order);
void pcsx2_perf_group_unregister_key(const void* key);

// ---- String utilities ------------------------------------------------------
bool pcsx2_string_strcasecmp(const char* a, const char* b);
uint32_t pcsx2_string_strlcpy(char* dst, uint32_t dst_len, const char* src);
int32_t pcsx2_string_strncasecmp(const char* a, const char* b, uint32_t n);
bool pcsx2_string_wildcard_match(const char* subject, const char* mask);

// ---- Threading -------------------------------------------------------------
uint64_t pcsx2_thread_get_cpu_time();
uint64_t pcsx2_thread_get_ticks_per_second();
void pcsx2_thread_set_name(const char* name);
void pcsx2_thread_sleep(uint32_t ms);
void pcsx2_thread_sleep_until(uint64_t ticks);

// OpaqueThreadHandle helpers (forward-declared OpaqueThreadHandle struct
// is replicated below as a private type because we do not pull in the
// cbindgen header).
struct OpaqueThreadHandle
{
	uintptr_t native_handle;
	uintptr_t native_id;
	uintptr_t _reserved;
};

void pcsx2_threading_thread_handle_copy(struct OpaqueThreadHandle* dst, const struct OpaqueThreadHandle* src);
void pcsx2_threading_thread_handle_destroy(struct OpaqueThreadHandle* h);
void pcsx2_threading_thread_handle_move(struct OpaqueThreadHandle* dst, struct OpaqueThreadHandle* src);
struct OpaqueThreadHandle* pcsx2_threading_thread_handle_new();
void pcsx2_threading_work_sema_wait_for_work_with_spin(void* sema);

} // extern "C"
