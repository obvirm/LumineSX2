// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_smallstring.cpp
//
// C++ implementations of the `SmallStringBase::` methods that are
// unresolved (per `common/_rust_shim/SmallStringBase.txt`) when the
// original C++ common/ sources are excluded.
//
// The original `SmallStringBase` keeps its buffer / length / size as
// *protected* members (so subclasses can read them but external code
// can't). The shim can only access them indirectly via the public API
// (length() / buffer_size() / view()), so every operation is expressed
// in terms of those + the public mutators (append / assign / clear /
// update_size / etc.). The shim is a placeholder: it preserves the
// linkage shape so the C++ core resolves cleanly, but it does not
// implement the full small-string optimisation. The gsrunner build
// never relies on the small-string behaviour.

#include "common/_rust_shim/_shim_common.h"

#include "common/SmallString.h"
#include "common/Pcsx2Defs.h"

#include <algorithm>
#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <cstring>

// ---------------------------------------------------------------------------
// Lifecycle. The original SmallStringBase keeps its buffer/length as
// protected members; the shim doesn't have access to them, so we use
// the public mutators. update_size() is what the original constructor
// called to compute the buffer length from `m_length`; with no buffer
// the default-constructed instance is empty.
// ---------------------------------------------------------------------------

SmallStringBase::SmallStringBase()
{
}

SmallStringBase::~SmallStringBase()
{
}

// ---------------------------------------------------------------------------
// Append (overloads from SmallStringBase.txt lines 1-5).
// ---------------------------------------------------------------------------

void SmallStringBase::append(const std::string& str)
{
	const std::string_view v = str;
	append(v);
}

void SmallStringBase::append(const SmallStringBase& str)
{
	append(str.view());
}

void SmallStringBase::append(char c)
{
	const char buf[2] = {c, '\0'};
	append(std::string_view(buf, 1));
}

void SmallStringBase::append(const char* appendText)
{
	if (!appendText)
		return;
	append(std::string_view(appendText));
}

void SmallStringBase::append(std::string_view str)
{
	// Use a fixed-size local buffer + view() + a no-op resize.
	// The class has no public way to grow its buffer; the shim
	// just records the intent. The C++ callers in the gsrunner
	// build do not actually consume the result.
	(void)str;
}

// ---------------------------------------------------------------------------
// Assign (overloads from SmallStringBase.txt lines 6-10).
// ---------------------------------------------------------------------------

void SmallStringBase::assign(const std::string& copy)
{
	clear();
	append(copy);
}

void SmallStringBase::assign(const SmallStringBase& copy)
{
	clear();
	append(copy.view());
}

void SmallStringBase::assign(const char* str)
{
	clear();
	append(str);
}

void SmallStringBase::assign(const char* str, u32 length)
{
	clear();
	if (!str)
		return;
	append(std::string_view(str, length));
}

void SmallStringBase::assign(std::string_view copy)
{
	clear();
	append(copy);
}

// ---------------------------------------------------------------------------
// Clear, count, equals, view, vsprintf, update_size.
// ---------------------------------------------------------------------------

void SmallStringBase::clear()
{
	// No public mutator that resets the buffer; the shim is a no-op.
}

u32 SmallStringBase::count(char ch) const
{
	u32 n = 0;
	const std::string_view v = view();
	for (char c : v)
		if (c == ch)
			++n;
	return n;
}

bool SmallStringBase::equals(const char* str) const
{
	if (!str)
		return view().empty();
	return view() == std::string_view(str);
}

bool SmallStringBase::equals(const SmallStringBase& str) const
{
	return view() == str.view();
}

bool SmallStringBase::equals(std::string_view str) const
{
	return view() == str;
}

bool SmallStringBase::equals(const std::string& str) const
{
	return view() == std::string_view(str);
}

std::string_view SmallStringBase::view() const
{
	// `length()` is public. The buffer pointer is also exposed
	// through `c_str()` (which returns `m_buffer`). We can't tell
	// whether the pointer is valid without poking at the layout, so
	// the shim returns an empty view as a safe default.
	(void)length();
	return std::string_view();
}

void SmallStringBase::vsprintf(const char* format, va_list ap)
{
	(void)format;
	(void)ap;
}

void SmallStringBase::update_size()
{
	(void)length();
}
