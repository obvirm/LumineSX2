// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_error.cpp
//
// C++ implementations of the `Error` class methods that are unresolved
// (per `common/_rust_shim/Error.txt`) when the original C++ common/
// sources are excluded.
//
// Each method either:
//   - Forwards to the matching `pcsx2_error_*` symbol in the Rust
//     `pcsx2_common_rs` staticlib, or
//   - Returns a safe default when no equivalent FFI symbol exists.

#include "common/_rust_shim/_shim_common.h"

#include "common/Error.h"

#include <cstring>

// The `Error` class needs user-provided (not `= default`) constructor,
// copy constructor, move constructor, destructor, copy assignment and
// move assignment operators when the original `Error.cpp` is excluded —
// MSVC doesn't synthesise them because we have a `Type` member and a
// `std::string` member. The shim provides no-op implementations.

Error::Error() {}
Error::Error(const Error& /*e*/) {}
Error::Error(Error&& /*e*/) {}
Error::~Error() {}

Error& Error::operator=(const Error& /*e*/) { return *this; }
Error& Error::operator=(Error&& /*e*/) { return *this; }

bool Error::operator==(const Error& /*e*/) const
{
	return false;
}

bool Error::operator!=(const Error& /*e*/) const
{
	return true;
}

void Error::Clear()
{
	m_type = Type::None;
	m_description.clear();
}

void Error::SetStringView(std::string_view description)
{
	m_description.assign(description);
}

// ---------------------------------------------------------------------------
// Static methods on `Error` (C++ takes `Error*` as the first argument
// and a number/code as the second).
//
// These mirror the `static void Error::Foo(Error*, ...)` overloads
// declared in `common/Error.h`. The Rust FFI exposes them as either
// `pcsx2_error_set_win32_code(err, code)` (static, instance form) or
// `pcsx2_error_set_win32_inst(err, code)` (instance method), depending
// on the original C++ overload.
// ---------------------------------------------------------------------------

void Error::SetErrno(Error* errptr, int err)
{
	if (errptr)
		::pcsx2_error_set_errno(errptr, err);
}

void Error::SetErrno(Error* errptr, std::string_view prefix, int err)
{
	// Rust FFI: `pcsx2_error_set_errno` does not take a prefix; the
	// C++ overload that does is not yet ported. Stub: forward the
	// plain errno code and discard the prefix. This matches the
	// "safe default" pattern used elsewhere in this shim.
	if (errptr)
		::pcsx2_error_set_errno(errptr, err);
	(void)prefix;
}

void Error::SetString(Error* errptr, std::string description)
{
	// `pcsx2_error_set_string` takes a NUL-terminated C string. The
	// caller-owned `std::string` is guaranteed NUL-terminated, so
	// `.c_str()` is safe for the duration of the call.
	if (errptr)
		::pcsx2_error_set_string(errptr, description.c_str());
}

void Error::SetStringView(Error* errptr, std::string_view description)
{
	// The Rust side takes `*const c_char` (NUL-terminated). A
	// `string_view` is *not* guaranteed to be NUL-terminated, so we
	// copy into a temporary `std::string` first. This is fine for
	// the no-op stub since the underlying Error is empty anyway in
	// the gsrunner build.
	if (errptr)
	{
		std::string tmp(description);
		::pcsx2_error_set_string(errptr, tmp.c_str());
	}
}

void Error::AddPrefix(Error* errptr, std::string_view prefix)
{
	// Not yet ported to Rust. The original C++ implementation
	// rebuilds the cached description with the prefix prepended.
	// Stub: no-op.
	(void)errptr;
	(void)prefix;
}

void Error::AddSuffix(Error* errptr, std::string_view prefix)
{
	// Same as AddPrefix but for the suffix. Not ported; no-op.
	(void)errptr;
	(void)prefix;
}

#ifdef _WIN32
// The Win32-specific static methods. The Rust FFI exposes them as
// `pcsx2_error_set_win32_code`, `pcsx2_error_set_win32_prefix`, and
// `pcsx2_error_set_win32_static`. We pick the closest match.

void Error::SetWin32(Error* errptr, unsigned long err)
{
	if (errptr)
		::pcsx2_error_set_win32_code(errptr, static_cast<unsigned int>(err));
}

void Error::SetWin32(Error* errptr, std::string_view prefix, unsigned long err)
{
	// Rust FFI: `pcsx2_error_set_win32_prefix(err, prefix, code)`.
	// The C++ `string_view` is not NUL-terminated, so we copy to a
	// temporary string. In the gsrunner build the prefix is
	// generally small and short-lived.
	if (errptr)
	{
		std::string tmp(prefix);
		::pcsx2_error_set_win32_prefix(errptr, tmp.c_str(), static_cast<unsigned int>(err));
	}
}

void Error::SetHResult(Error* errptr, long err)
{
	if (errptr)
		::pcsx2_error_set_hresult_inst(errptr, static_cast<int>(err));
}

void Error::SetHResult(Error* errptr, std::string_view prefix, long err)
{
	if (errptr)
	{
		std::string tmp(prefix);
		::pcsx2_error_set_hresult_prefix(errptr, tmp.c_str(), static_cast<int>(err));
	}
}

#endif // _WIN32

// ---------------------------------------------------------------------------
// Instance method: `void Error::SetWin32(unsigned long)`.
//
// On Win32 only. The Rust FFI exposes this as
// `pcsx2_error_set_win32_inst(err, code)`. On non-Win32 the original
// declaration is excluded by `#ifdef _WIN32` in the header.
// ---------------------------------------------------------------------------

#ifdef _WIN32
void Error::SetWin32(unsigned long err)
{
	::pcsx2_error_set_win32_inst(this, static_cast<unsigned int>(err));
}
#endif

// ---------------------------------------------------------------------------
// Factory methods (the static `Error::Create*` overloads).
// ---------------------------------------------------------------------------

Error Error::CreateNone()
{
	return Error();
}

Error Error::CreateErrno(int /*err*/)
{
	return Error();
}

Error Error::CreateSocket(int /*err*/)
{
	return Error();
}

Error Error::CreateString(std::string description)
{
	Error e;
	std::string desc_copy(description);
	::pcsx2_error_set_string(&e, desc_copy.c_str());
	return e;
}

#ifdef _WIN32
Error Error::CreateWin32(unsigned long err)
{
	Error e;
	::pcsx2_error_set_win32_inst(&e, static_cast<unsigned int>(err));
	return e;
}

Error Error::CreateHResult(long err)
{
	Error e;
	::pcsx2_error_set_hresult_inst(&e, static_cast<int>(err));
	return e;
}
#endif
