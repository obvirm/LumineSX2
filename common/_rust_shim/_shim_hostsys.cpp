// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_hostsys.cpp
//
// C++ implementations of the `HostSys::` free functions that are
// unresolved (per `common/_rust_shim/HostSys.txt`) when the original
// C++ common/ sources are excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/HostSys.h"

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <string>

namespace HostSys
{
	void* CreateSharedMemory(const char* /*name*/, std::size_t /*size*/)
	{
		return nullptr;
	}

	void DestroySharedMemory(void* /*ptr*/)
	{
	}

	std::string GetFileMappingName(const char* prefix)
	{
		return std::string(prefix ? prefix : "");
	}

	void MemProtect(void* /*baseaddr*/, std::size_t /*size*/, const PageProtectionMode& /*mode*/)
	{
		// The Rust side does not yet export a `MemProtect` helper.
		// The shim is a no-op stub.
	}
} // namespace HostSys
