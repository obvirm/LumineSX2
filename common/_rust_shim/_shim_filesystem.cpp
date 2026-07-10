// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_filesystem.cpp
// bumped force rebuild
//
// C++ implementations of the `FileSystem::` free functions that are
// unresolved (per `common/_rust_shim/FileSystem.txt`) when the
// original C++ common/ sources are excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/FileSystem.h"

#include <cstdio>
#include <filesystem>
#include <fstream>
#include <optional>
#include <vector>
#include <span>
#include <string>
#include <vector>

namespace FileSystem
{
	// -------------------------------------------------------------------------
	// Helpers
	// -------------------------------------------------------------------------

	// Copy `n` bytes from a Rust-allocated buffer (`malloc`-compatible)
	// into an `std::vector<u8>`, then free the buffer.
	static std::optional<std::vector<u8>> TakeRustBytes(uint8_t* p, std::size_t len)
	{
		if (!p)
			return std::nullopt;
		std::vector<u8> out(p, p + len);
		::std::free(p);
		return out;
	}

	// -------------------------------------------------------------------------
	// Functions with Rust FFI equivalents.
	// -------------------------------------------------------------------------

	bool FileExists(const char* path)
	{
		return ::pcsx2_file_exists(path);
	}

	bool DirectoryExists(const char* path)
	{
		return ::pcsx2_directory_exists(path);
	}

	bool DirectoryIsEmpty(const char* path)
	{
		return ::pcsx2_filesystem_directory_is_empty(path);
	}

	bool CreateDirectoryPath(const char* path, bool recursive, Error* /*error*/)
	{
		return ::pcsx2_create_directory(path, recursive);
	}

	bool DeleteFilePath(const char* path, Error* /*error*/)
	{
		return ::pcsx2_delete_file(path);
	}

	bool RenamePath(const char* OldPath, const char* NewPath, Error* /*error*/)
	{
		return ::pcsx2_rename_file(OldPath, NewPath);
	}

	std::string GetProgramPath()
	{
		char buf[4096];
		const uint32_t written = ::pcsx2_get_program_path(buf, sizeof(buf));
		if (written == 0)
			return std::string();
		return std::string(buf, (written < sizeof(buf)) ? written : sizeof(buf) - 1);
	}

	std::string GetWorkingDirectory()
	{
		char buf[4096];
		const uint32_t written = ::pcsx2_get_working_directory(buf, sizeof(buf));
		if (written == 0)
			return std::string();
		return std::string(buf, (written < sizeof(buf)) ? written : sizeof(buf) - 1);
	}

	std::optional<std::vector<u8>> ReadBinaryFile(const char* filename)
	{
		uint8_t* out = nullptr;
		uintptr_t out_len = 0;
		if (!::pcsx2_read_binary_file(filename, &out, &out_len))
			return std::nullopt;
		return TakeRustBytes(out, static_cast<std::size_t>(out_len));
	}

	bool WriteBinaryFile(const char* filename, const void* data, std::size_t data_length)
	{
		return ::pcsx2_write_binary_file(filename, static_cast<const uint8_t*>(data),
			static_cast<uintptr_t>(data_length));
	}

	std::size_t ReadFileWithProgress(std::FILE* fp, void* dst, std::size_t length,
		ProgressCallback* progress, Error* error, std::size_t /*chunk_size*/)
	{
		return static_cast<std::size_t>(::pcsx2_filesystem_read_file_with_progress(
			static_cast<void*>(fp), dst, static_cast<uint64_t>(length),
			static_cast<void*>(progress), static_cast<void*>(error), 0));
	}

	std::size_t ReadFileWithPartialProgress(std::FILE* fp, void* dst, std::size_t length,
		ProgressCallback* progress, int startPercent, int endPercent, Error* error,
		std::size_t /*chunk_size*/)
	{
		return static_cast<std::size_t>(::pcsx2_filesystem_read_file_with_partial_progress(
			static_cast<void*>(fp), dst, static_cast<uint64_t>(length),
			static_cast<void*>(progress), startPercent, endPercent,
			static_cast<void*>(error), 0));
	}

	std::FILE* OpenCFile(const char* filename, const char* mode, Error* /*error*/)
	{
		// Rust FFI does not have a dedicated `OpenCFile`. Fall back
		// to the platform `std::fopen`. The shim preserves the
		// linkage shape so the C++ call site resolves.
		return std::fopen(filename, mode);
	}

	// -------------------------------------------------------------------------
	// Functions without Rust FFI equivalents: stubs.
	// -------------------------------------------------------------------------

	bool CopyFilePath(const char* /*source*/, const char* /*destination*/, bool /*replace*/)
	{
		return false;
	}

	bool DeleteDirectory(const char* /*path*/)
	{
		return false;
	}

	bool EnsureDirectoryExists(const char* path, bool recursive, Error* /*error*/)
	{
		if (!path || !*path)
			return false;
		std::error_code ec;
		if (recursive)
		{
			std::filesystem::create_directories(path, ec);
			return !ec;
		}
		// Non-recursive: create the directory if it doesn't exist; if it
		// already exists, that's fine (return true).
		std::filesystem::create_directory(path, ec);
		if (ec)
			return false;
		// create_directory returns false if the directory already exists,
		// which is the success case for the caller.
		return true;
	}

	bool FindFiles(const char* /*path*/, const char* /*pattern*/, u32 /*flags*/,
		FindResultsArray* results, ProgressCallback* /*cancel*/)
	{
		if (results)
			results->clear();
		return false;
	}

	int FSeek64(std::FILE* fp, s64 offset, int whence)
	{
		return std::fseek(fp, static_cast<long>(offset), whence);
	}

	s64 FTell64(std::FILE* fp)
	{
		return static_cast<s64>(std::ftell(fp));
	}

	s64 FSize64(std::FILE* fp)
	{
		if (!fp)
			return -1;
		const long pos = std::ftell(fp);
		std::fseek(fp, 0, SEEK_END);
		const long end = std::ftell(fp);
		std::fseek(fp, pos, SEEK_SET);
		return static_cast<s64>(end);
	}

	s64 GetPathFileSize(const char* /*path*/)
	{
		return 0;
	}

	std::vector<std::string> GetRootDirectoryList()
	{
		return {};
	}

#ifdef _WIN32
	std::wstring GetWin32Path(std::string_view /*str*/)
	{
		return std::wstring();
	}
#endif

	std::optional<std::string> ReadFileToString(const char* /*filename*/)
	{
		return std::nullopt;
	}

	ManagedCFilePtr OpenManagedCFile(const char* /*filename*/, const char* /*mode*/, Error* /*error*/)
	{
		return ManagedCFilePtr();
	}

	ManagedCFilePtr OpenManagedCFileTryIgnoreCase(const char* /*filename*/, const char* /*mode*/, Error* /*error*/)
	{
		return ManagedCFilePtr();
	}

	int OpenFDFile(const char* /*filename*/, int /*flags*/, int /*mode*/, Error* /*error*/)
	{
		return -1;
	}

	ManagedCFilePtr OpenManagedSharedCFile(const char* /*filename*/, const char* /*mode*/,
		FileShareMode /*share_mode*/, Error* /*error*/)
	{
		return ManagedCFilePtr();
	}

	std::FILE* OpenSharedCFile(const char* /*filename*/, const char* /*mode*/,
		FileShareMode /*share_mode*/, Error* /*error*/)
	{
		return nullptr;
	}

	bool RecursiveDeleteDirectory(const char* /*path*/)
	{
		return false;
	}

	bool SetPathCompression(const char* /*path*/, bool /*enable*/)
	{
		return false;
	}

	bool StatFile(const char* /*path*/, FILESYSTEM_STAT_DATA* pStatData)
	{
		if (pStatData)
			std::memset(pStatData, 0, sizeof(*pStatData));
		return false;
	}

	bool StatFile(std::FILE* /*fp*/, FILESYSTEM_STAT_DATA* pStatData)
	{
		if (pStatData)
			std::memset(pStatData, 0, sizeof(*pStatData));
		return false;
	}

	bool StatFile(const char* /*path*/, struct stat* st)
	{
		if (st)
			std::memset(st, 0, sizeof(*st));
		return false;
	}

	std::span<const u8> MapBinaryFileForRead(const char* filename)
	{
		if (!filename || !*filename)
			return {};
		std::ifstream f(filename, std::ios::binary | std::ios::ate);
		if (!f)
			return {};
		const std::streamsize size = f.tellg();
		if (size <= 0)
			return {};
		f.seekg(0, std::ios::beg);
		// Heap-allocate so the returned span stays valid after we exit
		// this function. PCSX2 reads the font once at startup and never
		// frees it explicitly (the OS reclaims the heap on process exit).
		auto* buf = new std::vector<u8>(static_cast<size_t>(size));
		if (!f.read(reinterpret_cast<char*>(buf->data()), size))
		{
			delete buf;
			return {};
		}
		return std::span<const u8>(buf->data(), buf->size());
	}
} // namespace FileSystem
