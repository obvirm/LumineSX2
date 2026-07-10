// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_crashhandler.cpp
//
// C++ implementations of the `CrashHandler::` free functions that are
// unresolved (per `common/_rust_shim/CrashHandler.txt`) when the
// original C++ common/ sources are excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/CrashHandler.h"

#include <string>

namespace CrashHandler
{
	bool Install()
	{
		return ::pcsx2_crash_handler_install();
	}

	void SetWriteDirectory(std::string_view dump_directory)
	{
		std::string tmp(dump_directory);
		::pcsx2_crash_handler_set_write_directory(tmp.c_str());
	}

	void WriteDumpForCaller()
	{
		::pcsx2_crash_handler_write_dump();
	}
} // namespace CrashHandler
