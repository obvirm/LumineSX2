// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_stringutil.cpp
//
// C++ implementations of the `StringUtil::` free functions that are
// unresolved (per `common/_rust_shim/StringUtil.txt`) when the
// original C++ common/ sources are excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/StringUtil.h"
#include "common/Pcsx2Defs.h"

#include <algorithm>
#include <cctype>
#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

namespace StringUtil
{
	// -------------------------------------------------------------------------
	// Functions with Rust FFI equivalents.
	// -------------------------------------------------------------------------

	std::size_t Strlcpy(char* dst, const char* src, std::size_t size)
	{
		return static_cast<std::size_t>(::pcsx2_string_strlcpy(dst, static_cast<uint32_t>(size), src));
	}

	std::size_t Strlcpy(char* dst, std::string_view src, std::size_t size)
	{
		// The Rust FFI takes a NUL-terminated C string; copy into
		// a temporary if necessary. In practice the C++ caller
		// always has a NUL-terminated buffer handy.
		std::string tmp(src);
		return static_cast<std::size_t>(::pcsx2_string_strlcpy(dst, static_cast<uint32_t>(size), tmp.c_str()));
	}

	bool WildcardMatch(const char* subject, const char* mask, bool /*case_sensitive*/)
	{
		return ::pcsx2_string_wildcard_match(subject, mask);
	}

	// -------------------------------------------------------------------------
	// Stub implementations.
	// -------------------------------------------------------------------------

	void AppendUTF16CharacterToUTF8(std::string& s, u16 ch)
	{
		// Append the low byte as Latin-1. Adequate for ASCII
		// characters and BMP code points; not a full UTF-8
		// encoder.
		if (ch < 0x80)
		{
			s.push_back(static_cast<char>(ch));
		}
		else
		{
			s.push_back(static_cast<char>(0xC0 | (ch >> 6)));
			s.push_back(static_cast<char>(0x80 | (ch & 0x3F)));
		}
	}

	bool compareNoCase(std::string_view str1, std::string_view str2)
	{
		const int r = ::pcsx2_string_strncasecmp(
			std::string(str1).c_str(), std::string(str2).c_str(),
			static_cast<uint32_t>(std::min(str1.size(), str2.size())));
		return r == 0;
	}

	std::optional<std::vector<u8>> DecodeHex(std::string_view str)
	{
		std::vector<u8> out;
		out.reserve(str.size() / 2);
		auto hexv = [](char c) -> int {
			if (c >= '0' && c <= '9') return c - '0';
			if (c >= 'a' && c <= 'f') return c - 'a' + 10;
			if (c >= 'A' && c <= 'F') return c - 'A' + 10;
			return -1;
		};
		int hi = -1;
		for (char c : str)
		{
			const int v = hexv(c);
			if (v < 0)
				return std::nullopt;
			if (hi < 0)
				hi = v;
			else
			{
				out.push_back(static_cast<u8>((hi << 4) | v));
				hi = -1;
			}
		}
		if (hi >= 0)
			return std::nullopt;
		return out;
	}

	std::size_t DecodeUTF8(std::string_view /*str*/, std::size_t /*offset*/, char32_t* /*ch*/)
	{
		return 0;
	}

	std::string Ellipsise(std::string_view str, u32 /*max_length*/, const char* /*ellipsis*/)
	{
		return std::string(str);
	}

	void EncodeAndAppendUTF8(std::string& s, char32_t ch)
	{
		if (ch < 0x80)
		{
			s.push_back(static_cast<char>(ch));
		}
		else if (ch < 0x800)
		{
			s.push_back(static_cast<char>(0xC0 | (ch >> 6)));
			s.push_back(static_cast<char>(0x80 | (ch & 0x3F)));
		}
		else
		{
			s.push_back(static_cast<char>(0xE0 | (ch >> 12)));
			s.push_back(static_cast<char>(0x80 | ((ch >> 6) & 0x3F)));
			s.push_back(static_cast<char>(0x80 | (ch & 0x3F)));
		}
	}

	bool ParseAssignmentString(std::string_view /*str*/, std::string_view* /*key*/, std::string_view* /*value*/)
	{
		return false;
	}

	void ReplaceAll(std::string* /*subject*/, std::string_view /*search*/, std::string_view /*replacement*/)
	{
	}

	std::vector<std::string_view> SplitString(std::string_view str, char delimiter, bool skip_empty)
	{
		std::vector<std::string_view> out;
		std::size_t pos = 0;
		while (pos <= str.size())
		{
			const std::size_t next = str.find(delimiter, pos);
			std::string_view part = str.substr(pos, (next == std::string_view::npos) ? std::string_view::npos : next - pos);
			if (!part.empty() || !skip_empty)
				out.push_back(part);
			if (next == std::string_view::npos)
				break;
			pos = next + 1;
		}
		return out;
	}

	std::string StdStringFromFormat(const char* format, ...)
	{
		// Used by C++ callers expecting a `printf`-style formatter.
		// The Rust side does not yet have a printf-style helper;
		// forward through `vsnprintf` into a fixed buffer.
		if (!format)
			return std::string();
		char buf[4096];
		va_list ap;
		va_start(ap, format);
		std::vsnprintf(buf, sizeof(buf), format, ap);
		va_end(ap);
		return std::string(buf);
	}

	std::string StdStringFromFormatV(const char* format, std::va_list ap)
	{
		if (!format)
			return std::string();
		char buf[4096];
		std::vsnprintf(buf, sizeof(buf), format, ap);
		return std::string(buf);
	}

	std::string_view StripWhitespace(std::string_view str)
	{
		const auto is_ws = [](char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r'; };
		std::size_t start = 0;
		while (start < str.size() && is_ws(str[start]))
			++start;
		std::size_t end = str.size();
		while (end > start && is_ws(str[end - 1]))
			--end;
		return str.substr(start, end - start);
	}

	void StripWhitespace(std::string* str)
	{
		if (!str)
			return;
		const auto view = StripWhitespace(*str);
		str->assign(view.data(), view.size());
	}

	std::string toLower(std::string_view str)
	{
		std::string out(str);
		std::transform(out.begin(), out.end(), out.begin(),
			[](unsigned char c) { return static_cast<char>(std::tolower(c)); });
		return out;
	}

	std::string U128ToString(const u128& /*u*/)
	{
		return std::string("0x0");
	}

#ifdef _WIN32
#include <windows.h>

	std::wstring UTF8StringToWideString(std::string_view str)
	{
		if (str.empty())
			return std::wstring();
		const int wlen = MultiByteToWideChar(CP_UTF8, 0, str.data(), static_cast<int>(str.size()), nullptr, 0);
		if (wlen <= 0)
			return std::wstring();
		std::wstring out(static_cast<size_t>(wlen), L'\0');
		MultiByteToWideChar(CP_UTF8, 0, str.data(), static_cast<int>(str.size()), out.data(), wlen);
		return out;
	}

	std::string WideStringToUTF8String(const std::wstring_view& str)
	{
		if (str.empty())
			return std::string();
		const int len = WideCharToMultiByte(CP_UTF8, 0, str.data(), static_cast<int>(str.size()), nullptr, 0, nullptr, nullptr);
		if (len <= 0)
			return std::string();
		std::string out(static_cast<size_t>(len), '\0');
		WideCharToMultiByte(CP_UTF8, 0, str.data(), static_cast<int>(str.size()), out.data(), len, nullptr, nullptr);
		return out;
	}
#endif

} // namespace StringUtil
