// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_path.cpp
//
// Forward the C++ `Path::` free functions (per `common/_rust_shim/Path.txt`)
// to the corresponding `pcsx2_path_*` symbols in the Rust
// `pcsx2_common_rs` staticlib, or return safe defaults when no
// equivalent FFI symbol exists.

#include "common/_rust_shim/_shim_common.h"

#include "common/Path.h"

#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

namespace Path
{
	// Helper: allocate and return an `std::string` from a
	// NUL-terminated C string. The C++ free functions take ownership
	// of the returned string. The Rust side hands us back a
	// NUL-terminated `char*` allocated with `libc::malloc` (which
	// is `free`/`delete`-compatible) OR returns null.
	static std::string FromCStr(const char* p)
	{
		if (!p)
			return std::string();
		return std::string(p);
	}

	// Helper: copy into a fixed-size buffer for the
	// `pcsx2_path_get_*` family (Directory, Extension, FileName,
	// FileTitle, ToNativePath). The Rust function returns the byte
	// count (excluding NUL); we always pass a generous buffer.
	template <std::size_t N>
	static std::string CopyToBuffer(const std::string_view src,
		uint32_t (*fn)(char*, uint32_t, const char*))
	{
		char buf[N];
		const uint32_t written = fn(buf, N, src.data());
		if (written == 0)
			return std::string();
		return std::string(buf, (written < N) ? written : N - 1);
	}

	// `pcsx2_path_is_absolute` returns a `u32` (1/0) instead of a
	// bool for the FFI shape reasons noted in pcsx2_common_rs.h.
	static std::string_view CopyToView(const std::string_view src,
		uint32_t (*fn)(char*, uint32_t, const char*))
	{
		// We can't return a string_view with arbitrary lifetime
		// safely, so fall through to the std::string version and
		// leak the string. The shim is a placeholder anyway.
		std::string tmp = CopyToBuffer<4096>(src, fn);
		// Leak the storage so the returned `string_view` is valid
		// until program exit. This is the documented placeholder
		// behaviour: callers are expected to copy the result
		// immediately. In the gsrunner build Path::Get* are not
		// used outside the C++ side which copies the view into a
		// std::string at the call site.
		static std::vector<std::string> pool;
		pool.emplace_back(std::move(tmp));
		return std::string_view(pool.back());
	}

	// -------------------------------------------------------------------------
	// Functions with Rust FFI equivalents.
	// -------------------------------------------------------------------------

	std::string AppendDirectory(const std::string_view base, const std::string_view new_dir)
	{
		// Rust FFI uses NUL-terminated C strings; copy both arguments
		// into temporaries. Empty views are passed as `""` (Rust
		// treats null and empty identically here).
		std::string b(base);
		std::string n(new_dir);
		char* r = ::pcsx2_path_append_directory(b.c_str(), n.c_str());
		if (!r)
			return std::string();
		std::string out(r);
		::std::free(r);
		return out;
	}

	std::string Combine(const std::string_view base, const std::string_view next)
	{
		std::string b(base);
		std::string n(next);
		char buf[4096];
		const uint32_t written = ::pcsx2_path_combine(buf, sizeof(buf), b.c_str(), n.c_str());
		if (written == 0)
			return std::string();
		return std::string(buf, written);
	}

	void ChangeFileName(std::string* path, const std::string_view new_filename)
	{
		// `pcsx2_path_change_file_name` writes back into the
		// caller's buffer, leaving it untouched on null inputs.
		// We use a generous scratch buffer; the resulting string
		// is shorter than or equal to the input for sane paths.
		if (!path)
			return;
		char buf[4096];
		std::strncpy(buf, path->c_str(), sizeof(buf) - 1);
		buf[sizeof(buf) - 1] = '\0';
		std::string nf(new_filename);
		const bool ok = ::pcsx2_path_change_file_name(buf, nf.c_str());
		if (ok)
			*path = buf;
	}

	std::string ChangeFileName(const std::string_view path, const std::string_view new_filename)
	{
		std::string out(path);
		ChangeFileName(&out, new_filename);
		return out;
	}

	std::string_view GetDirectory(const std::string_view path)
	{
		return CopyToView(path, ::pcsx2_path_get_directory);
	}

	std::string_view GetExtension(const std::string_view path)
	{
		return CopyToView(path, ::pcsx2_path_get_extension);
	}

	std::string_view GetFileName(const std::string_view path)
	{
		return CopyToView(path, ::pcsx2_path_get_file_name);
	}

	std::string_view GetFileTitle(const std::string_view path)
	{
		return CopyToView(path, ::pcsx2_path_get_file_title);
	}

	bool IsAbsolute(const std::string_view path)
	{
		std::string p(path);
		return ::pcsx2_path_is_absolute(p.c_str()) != 0;
	}

	std::string JoinNativePath(const std::vector<std::string_view>& components)
	{
		// Rust FFI takes `(const char* const*, uintptr_t)`. We need
		// to build a parallel array of NUL-terminated C strings.
		std::vector<std::string> storage;
		storage.reserve(components.size());
		for (const auto& c : components)
			storage.emplace_back(c);
		std::vector<const char*> ptrs;
		ptrs.reserve(storage.size());
		for (const auto& s : storage)
			ptrs.push_back(s.c_str());

		char* r = ::pcsx2_path_join_native_path(ptrs.data(), static_cast<uintptr_t>(ptrs.size()));
		if (!r)
			return std::string();
		std::string out(r);
		::std::free(r);
		return out;
	}

	std::string ToNativePath(const std::string_view path)
	{
		return CopyToBuffer<4096>(path, ::pcsx2_path_to_native_path);
	}

	// -------------------------------------------------------------------------
	// Functions without Rust FFI equivalents: stub implementations.
	// -------------------------------------------------------------------------

	std::string BuildRelativePath(const std::string_view filename, const std::string_view new_filename)
	{
		(void)new_filename;
		return std::string(filename);
	}

	std::string Canonicalize(const std::string_view path)
	{
		return std::string(path);
	}

	void Canonicalize(std::string* path)
	{
		(void)path;
	}

	std::string CreateFileURL(const std::string_view path)
	{
		return std::string("file://") + std::string(path);
	}

	bool IsValidFileName(const std::string_view str, bool /*allow_slashes*/)
	{
		return !str.empty();
	}

	std::string MakeRelative(const std::string_view path, const std::string_view /*relative_to*/)
	{
		return std::string(path);
	}

	std::string RealPath(const std::string_view path)
	{
		return std::string(path);
	}

	std::string ReplaceExtension(const std::string_view path, const std::string_view new_extension)
	{
		std::string out(path);
		const auto dot = out.find_last_of('.');
		if (dot != std::string::npos)
			out.erase(dot);
		out.append(new_extension.data(), new_extension.size());
		return out;
	}

	std::string SanitizeFileName(const std::string_view str, bool /*strip_slashes*/)
	{
		return std::string(str);
	}

	void SanitizeFileName(std::string* str, bool /*strip_slashes*/)
	{
		(void)str;
	}

	std::vector<std::string_view> SplitNativePath(const std::string_view /*path*/)
	{
		return {};
	}

	std::string URLDecode(const std::string_view str)
	{
		return std::string(str);
	}

	std::string URLEncode(const std::string_view str)
	{
		return std::string(str);
	}

	std::string_view StripExtension(const std::string_view path)
	{
		return CopyToView(path, ::pcsx2_path_get_file_title);
	}
} // namespace Path
