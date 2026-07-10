// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

// _shim_httpdownloader.cpp
//
// C++ implementations of the `HTTPDownloader::` methods that are
// unresolved (per `common/_rust_shim/HTTPDownloader.txt`) when the
// original C++ common/ sources are excluded.

#include "common/_rust_shim/_shim_common.h"

#include "common/HTTPDownloader.h"

#include <memory>
#include <string>

HTTPDownloader::HTTPDownloader() = default;
HTTPDownloader::~HTTPDownloader() = default;

const char HTTPDownloader::DEFAULT_USER_AGENT[] = "pcsx2/1.0";

std::unique_ptr<HTTPDownloader> HTTPDownloader::Create(std::string /*user_agent*/)
{
	// No Rust FFI for the factory; return a null unique_ptr.
	return nullptr;
}

std::string HTTPDownloader::GetExtensionForContentType(const std::string& /*content_type*/)
{
	return std::string();
}

void HTTPDownloader::SetTimeout(float /*timeout*/)
{
}

void HTTPDownloader::SetMaxActiveRequests(u32 /*max_active_requests*/)
{
}

void HTTPDownloader::CreateRequest(std::string /*url*/, Request::Callback /*callback*/,
	ProgressCallback* /*progress*/)
{
}

void HTTPDownloader::CreatePostRequest(std::string /*url*/, std::string /*post_data*/,
	Request::Callback /*callback*/, ProgressCallback* /*progress*/)
{
}

void HTTPDownloader::PollRequests()
{
}

void HTTPDownloader::WaitForAllRequests()
{
}

bool HTTPDownloader::HasAnyRequests()
{
	return false;
}
