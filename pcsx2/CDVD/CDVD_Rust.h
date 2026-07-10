// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

#pragma once

#include <cstdint>

// Opaque handle to Rust CDVDReader
struct CDVDReaderHandle;

extern "C" {

/// Open a CDVD image file (ISO, CHD, CSO, etc.)
/// Returns NULL on error
CDVDReaderHandle* pcsx2_cdvd_open(const char* path);

/// Close and free a CDVD reader
void pcsx2_cdvd_close(CDVDReaderHandle* handle);

/// Read sectors from the disc
/// Returns number of bytes read, or -1 on error
int pcsx2_cdvd_read_sectors(
    CDVDReaderHandle* handle,
    uint32_t lsn,
    uint32_t sector_count,
    uint8_t* buffer);

/// Get the total size of the disc in bytes
uint64_t pcsx2_cdvd_get_size(CDVDReaderHandle* handle);

/// Get the sector count (size / 2048)
uint32_t pcsx2_cdvd_get_sector_count(CDVDReaderHandle* handle);

} // extern "C"
