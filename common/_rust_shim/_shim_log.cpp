// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_log.cpp
//
// C++ implementations of the `Log::` free functions that are unresolved
// (per `common/_rust_shim/Log.txt`) when the original C++ common/
// sources are excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/Console.h"

#include <cstdarg>
#include <cstdint>
#include <cstring>
#include <string>

namespace Log
{
	// -------------------------------------------------------------------------
	// The Rust FFI uses a packed (LogLevel, ConsoleColor) representation.
	// The C++ uses the (LOGLEVEL, ConsoleColors) enums; they share the
	// numeric values (None=0 ... Trace=6) so the cast is safe.
	// -------------------------------------------------------------------------

	static ::pcsx2::LogLevel ToRs(LOGLEVEL level)
	{
		return static_cast<::pcsx2::LogLevel>(static_cast<uint32_t>(level));
	}

	static ::pcsx2::ConsoleColor ToRs(ConsoleColors color)
	{
		return static_cast<::pcsx2::ConsoleColor>(static_cast<uint32_t>(color));
	}

	LOGLEVEL GetMaxLevel()
	{
		return static_cast<LOGLEVEL>(static_cast<uint32_t>(::pcsx2_log_get_level()));
	}

	bool IsConsoleOutputEnabled()
	{
		// No dedicated FFI; return false by default. The Rust side
		// doesn't expose this query yet.
		return false;
	}

	bool IsFileOutputEnabled()
	{
		return false;
	}

	void SetConsoleOutputLevel(LOGLEVEL level)
	{
		::pcsx2_log_set_level(ToRs(level));
	}

	void SetDebugOutputLevel(LOGLEVEL /*level*/)
	{
		// Not yet ported; no-op.
	}

	bool SetFileOutputLevel(LOGLEVEL /*level*/, std::string /*path*/)
	{
		return false;
	}

	void SetTimestampsEnabled(bool /*enabled*/)
	{
		// No-op stub.
	}

	void Write(LOGLEVEL level, ConsoleColors color, std::string_view message)
	{
		// The Rust FFI takes `(LogLevel, ConsoleColor, const char*, uintptr_t)`.
		// The C++ `string_view` is not NUL-terminated, so we hand over
		// `(data, size)` explicitly.
		std::string tmp(message); // ensure NUL-terminated for safety
		::pcsx2_log_write(ToRs(level), ToRs(color),
			tmp.c_str(), static_cast<uintptr_t>(tmp.size()));
	}

	void Writev(LOGLEVEL level, ConsoleColors color, const char* format, std::va_list ap)
	{
		if (!format)
			return;
		char buf[4096];
		std::vsnprintf(buf, sizeof(buf), format, ap);
		std::string s(buf);
		::pcsx2_log_write(ToRs(level), ToRs(color),
			s.c_str(), static_cast<uintptr_t>(s.size()));
	}

	void WriteFmtArgs(LOGLEVEL /*level*/, ConsoleColors /*color*/, fmt::string_view /*fmt*/,
		fmt::format_args /*args*/)
	{
		// Not ported to Rust. The Rust side doesn't yet expose a
		// fmt-args style entry point. No-op stub.
	}

} // namespace Log
