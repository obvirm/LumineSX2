// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

#include "PrecompiledHeader.h"
#include "CDVD_Rust.h"
#include "ThreadedFileReader.h"
#include "common/Console.h"
#include "common/Error.h"
#include <memory>

/// File reader that uses Rust CDVD implementation
/// Currently supports ISO files (CHD/CSO coming soon)
class RustFileReader final : public ThreadedFileReader
{
	DeclareNoncopyableObject(RustFileReader);

public:
	RustFileReader();
	~RustFileReader() override;

	bool Open2(std::string filename, Error* error) override;
	bool Precache2(ProgressCallback* progress, Error* error) override;
	Chunk ChunkForOffset(u64 offset) override;
	int ReadChunk(void* dst, s64 blockID) override;
	void Close2() override;
	uint GetBlockCount() const override;

private:
	CDVDReaderHandle* m_handle = nullptr;
	u64 m_size = 0;
};

RustFileReader::RustFileReader() = default;

RustFileReader::~RustFileReader()
{
	Close2();
}

bool RustFileReader::Open2(std::string filename, Error* error)
{
	m_filename = std::move(filename);
	
	Console.WriteLn(Color_Green, "CDVD: Opening via Rust reader: %s", m_filename.c_str());
	
	m_handle = pcsx2_cdvd_open(m_filename.c_str());
	if (!m_handle)
	{
		Error::SetString(error, fmt::format("Failed to open '{}' via Rust CDVD reader", m_filename));
		return false;
	}
	
	m_size = pcsx2_cdvd_get_size(m_handle);
	m_blocksize = 2048;
	
	Console.WriteLn(Color_Green, "CDVD: Rust reader opened successfully, size: %llu bytes (%u sectors)",
		m_size, GetBlockCount());
	
	return true;
}

bool RustFileReader::Precache2(ProgressCallback* progress, Error* error)
{
	// No precaching for now
	return true;
}

ThreadedFileReader::Chunk RustFileReader::ChunkForOffset(u64 offset)
{
	Chunk chunk = {};
	chunk.chunkID = static_cast<s64>(offset / m_blocksize);
	chunk.offset = offset;
	chunk.length = m_blocksize;
	return chunk;
}

int RustFileReader::ReadChunk(void* dst, s64 blockID)
{
	if (blockID < 0 || !m_handle)
		return -1;
	
	static u64 s_readCount = 0;
	s_readCount++;
	if ((s_readCount % 1000) == 0)
		Console.WriteLn(Color_Green, "CDVD: Rust reader sectors read so far: %llu", s_readCount);
	
	const u32 lsn = static_cast<u32>(blockID);
	const int result = pcsx2_cdvd_read_sectors(m_handle, lsn, 1, static_cast<u8*>(dst));
	
	if (result < 0)
	{
		Console.Error("CDVD: Rust reader failed to read sector %u", lsn);
		return -1;
	}
	
	return result;
}

void RustFileReader::Close2()
{
	if (m_handle)
	{
		Console.WriteLn("CDVD: Closing Rust reader");
		pcsx2_cdvd_close(m_handle);
		m_handle = nullptr;
	}
	m_size = 0;
}

uint RustFileReader::GetBlockCount() const
{
	return static_cast<uint>(m_size / m_blocksize);
}

// Factory function to create a Rust file reader
std::unique_ptr<ThreadedFileReader> CreateRustFileReader()
{
	return std::make_unique<RustFileReader>();
}
