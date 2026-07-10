# Plan — common/ Full Coverage Analysis

## Goal
Analyze every remaining C++ file in `common/` vs its Rust equivalent in `rust/common/src/`. Verify that all C++ functionality is properly ported.

## Groups

### Group A: Core Headers (16 header-only files)
AlignedMalloc.h, BitUtils.h, ByteSwap.h, Easing.h, EnumOps.h, FPControl.h, HashCombine.h, HeapArray.h, HeterogeneousContainers.h, Pcsx2Defs.h, Pcsx2Types.h, ScopedGuard.h, SingleRegisterTypes.h, VectorIntrin.h, WrappedMemCopy.h, MRCHelpers.h (skip - macOS ObjC)

### Group B: String + Path + File (6 files)
StringUtil.cpp/h, SmallString.cpp/h, Path.h, FileSystem.cpp/h

### Group C: Error + Crash + Assert + FastJmp (8 files)
Error.cpp/h, CrashHandler.cpp/h, Assertions.cpp/h, FastJmp.cpp/h

### Group D: Settings + Memory I/F (6 files)
SettingsInterface.h, SettingsWrapper.cpp/h, MemoryInterface.cpp/h, MemorySettingsInterface.cpp/h

### Group E: Thread/Timer/Sema (5 files)
Threading.h, Timer.cpp/h, Semaphore.cpp, ReadbackSpinManager.cpp/h

### Group F: I/O + YAML + Console (9 files)
WAVWriter.cpp/h, WindowInfo.cpp/h, TextureDecompress.cpp/h, YAML.cpp/h, Console.cpp/h

### Group G: HTTP + MD5 + Cache + Progress + Perf + Zip (11 files)
HTTPDownloader.cpp/h, HTTPDownloaderCurl.cpp/h, HTTPDownloaderWinHTTP.cpp/h, MD5Digest.cpp/h, LRUCache.h, ProgressCallback.cpp/h, Perf.cpp/h, ZipHelpers.h

### Group H: HostSys + Platform (7 files)
HostSys.cpp/h, DarwinMisc.cpp/h, DarwinThreads.cpp, LnxHostSys.cpp, LnxMisc.cpp, LnxThreads.cpp, CocoaTools.h

## Output Format
For each C++ function/class:
- ✅ COVERED — Rust equivalent exists
- ❌ MISSING — Rust equivalent missing
- ⚠️ PARTIAL — Rust exists but incomplete

Write report to `analysis/agent{X}_{group}.md`
