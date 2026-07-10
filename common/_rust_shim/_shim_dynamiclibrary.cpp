// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_dynamiclibrary.cpp
//
// C++ implementations of the `DynamicLibrary::` methods that are
// unresolved (per `common/_rust_shim/DynamicLibrary.txt`) when the
// original C++ common/ sources are excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/DynamicLibrary.h"

#include <cstdint>
#include <cstdio>
#include <string>

DynamicLibrary::DynamicLibrary() = default;
DynamicLibrary::DynamicLibrary(const char* /*filename*/) {}
DynamicLibrary::DynamicLibrary(DynamicLibrary&& /*move*/) = default;
DynamicLibrary::~DynamicLibrary() = default;

std::string DynamicLibrary::GetUnprefixedFilename(const char* filename)
{
	return std::string(filename ? filename : "");
}

std::string DynamicLibrary::GetVersionedFilename(const char* libname, int major, int minor)
{
	// Not ported to Rust; reproduce the platform-agnostic shape.
	std::string out(libname ? libname : "");
	if (major >= 0 && minor >= 0)
	{
		char buf[64];
		std::snprintf(buf, sizeof(buf), ".%d.%d", major, minor);
		out += buf;
	}
	return out;
}

bool DynamicLibrary::Open(const char* /*filename*/, Error* /*error*/)
{
	return false;
}

void DynamicLibrary::Adopt(void* handle)
{
	m_handle = handle;
}

void DynamicLibrary::Close()
{
	m_handle = nullptr;
}

void* DynamicLibrary::GetSymbolAddress(const char* /*name*/) const
{
	return nullptr;
}

DynamicLibrary& DynamicLibrary::operator=(DynamicLibrary&& /*move*/) = default;
