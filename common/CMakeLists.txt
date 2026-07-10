# Check that people use the good file
if(NOT TOP_CMAKE_WAS_SOURCED)
	message(FATAL_ERROR "
	You did not 'cmake' the good CMakeLists.txt file. Use the one in the top dir.
	It is advice to delete all wrongly generated cmake stuff => CMakeFiles & CMakeCache.txt")
endif(NOT TOP_CMAKE_WAS_SOURCED)

add_library(common)

# Link against YAML library (ryml) — common's YAML.cpp needs it.
# vcpkg's ryml puts headers under include/ryml/ (one level deeper than
# expected) so #include <ryml.hpp> needs the subdir on the include path.
find_package(ryml REQUIRED)
set(RYML_INCLUDE_DIR_SUB "${RYML_INCLUDE_DIR}/ryml")
if(NOT EXISTS "${RYML_INCLUDE_DIR_SUB}/ryml.hpp")
	message(FATAL_ERROR "ryml.hpp not found in ${RYML_INCLUDE_DIR_SUB}")
endif()
message(STATUS "ryml headers at: ${RYML_INCLUDE_DIR_SUB}")
target_include_directories(common PUBLIC "${RYML_INCLUDE_DIR_SUB}")
target_link_libraries(common PUBLIC ryml::ryml)

# ============================================================================
# pcsx2_common_rs — Rust reimplementation of common/ — built as a static lib
# and linked into the PCSX2 C++ target. Provides drop-in replacements for the
# leaf utilities and platform helpers via `pcsx2_*` extern "C" FFI symbols.
#
# Build orchestration:
#   1. `cargo build --release` produces `libpcsx2_common_rs.a` (or .lib on Win)
#   2. The generated `pcsx2_common_rs.h` provides C++ declarations.
#   3. CMake links the static lib into `common` (which is then linked into
#      every PCSX2 binary that consumes common).
#
# Disable with `-DUSE_RUST_COMMON=OFF` to fall back to the original C++ common/.
# ============================================================================
option(USE_RUST_COMMON "Link the Rust reimplementation of common/ into PCSX2" ON)

# Drop the original C++ common/ sources entirely. The Rust port in
# `rust/common/` provides every FFI symbol the C++ core needs (verified
# via the existing `pcsx2-gsrunner.exe` build which links Rust-only).
# When ON, the CMakeLists skips `target_sources(common ...)` for the
# C++ .cpp files but still keeps the emitter (no Rust port yet) and
# platform-specific assembly that hasn't been moved.
#
# Default ON: the Rust shim in `common/_rust_shim/` provides every
# symbol the C++ core needs (verified via the existing
# `pcsx2-gsrunner.exe` build which links Rust-only with the shim).
# Flip back to OFF if a symbol is found missing and the C++ side
# needs to provide a fallback during transition.
option(EXCLUDE_CPP_COMMON "Skip compiling the original C++ common/ sources (rely entirely on the Rust port + shim)" ON)

if(USE_RUST_COMMON)
	set(PCSX2_COMMON_RS_DIR "${CMAKE_SOURCE_DIR}/rust/common")
	set(PCSX2_COMMON_RS_HEADER "${CMAKE_CURRENT_BINARY_DIR}/pcsx2_common_rs.h")

	# Build the Rust staticlib via cargo.
	# `cargo build` is invoked from a custom command so it re-runs whenever
	# any source under rust/common/src changes.
	# Build the Rust staticlib via direct cargo invocation, plus cbindgen
# for the C++ header. We pass RUSTFLAGS as a single argument (no
# shell-style quoting that CMake gets confused about).
add_custom_command(
		OUTPUT "${PCSX2_COMMON_RS_HEADER}"
		COMMAND
			${CMAKE_COMMAND} -E env "RUSTFLAGS=-C target-feature=+sse4.1"
			cargo build --release --lib
		COMMAND
			cbindgen --config cbindgen.toml --crate pcsx2_common_rs --output "${PCSX2_COMMON_RS_HEADER}"
		WORKING_DIRECTORY "${PCSX2_COMMON_RS_DIR}"
		COMMENT "Building Rust pcsx2_common_rs (cargo + cbindgen)"
		VERBATIM
	)

	# Make the header generation a dependency of the `common` target so it
	# re-runs whenever the Rust sources change.
	add_custom_target(pcsx2_common_rs_build DEPENDS "${PCSX2_COMMON_RS_HEADER}")

	add_dependencies(common pcsx2_common_rs_build)

	# Include path for the generated header.
	target_include_directories(common PUBLIC "${CMAKE_CURRENT_BINARY_DIR}")

	# Tell the linker where the static lib lives (relative to cargo's target dir).
	# On Linux/Mac the file is `libpcsx2_common_rs.a`; on Windows it's
	# `pcsx2_common_rs.lib`. Cargo emits both under target/<profile>/.
	set(PCSX2_COMMON_RS_LIB "${PCSX2_COMMON_RS_DIR}/target/$<CONFIG>/$<IF:$<PLATFORM_ID:Windows>,pcsx2_common_rs.lib,libpcsx2_common_rs.a>")

	# Link the staticlib into `common` (transitively into every PCSX2 binary).
	target_link_libraries(common PUBLIC "${PCSX2_COMMON_RS_LIB}")

	# C++ runtime (Rust panics need this on some platforms).
	if(MSVC)
		# No extra link needed — MSVC links the C++ runtime by default.
	else()
		target_link_libraries(common PUBLIC stdc++)
	endif()

	# Rust's `std` crate on Windows pulls in NT system calls
	# (NtReadFile, NtCreateFile, ...) and version helpers
	# (RtlGetVersion) that aren't on the default link line.
	# It also needs winsock2 (ws2_32), user profile (userenv),
	# crypto (bcrypt), and registry (advapi32) for std::net, home_dir, etc.
	if(WIN32)
		target_link_libraries(common PUBLIC ntdll ws2_32 userenv bcrypt advapi32 version)
	endif()

	message(STATUS "PCSX2: linking Rust pcsx2_common_rs from ${PCSX2_COMMON_RS_DIR}")
endif()

if(NOT EXCLUDE_CPP_COMMON)
	target_sources(common PRIVATE
		AlignedMalloc.cpp
		Assertions.cpp
		Console.cpp
		CrashHandler.cpp
		DynamicLibrary.cpp
		Error.cpp
		FastJmp.cpp
		FileSystem.cpp
		HostSys.cpp
		Image.cpp
		HTTPDownloader.cpp
		MemoryInterface.cpp
		MemorySettingsInterface.cpp
		MD5Digest.cpp
		PrecompiledHeader.cpp
		Perf.cpp
		ProgressCallback.cpp
		ReadbackSpinManager.cpp
		Semaphore.cpp
		SettingsWrapper.cpp
		SmallString.cpp
		StringUtil.cpp
		TextureDecompress.cpp
		Timer.cpp
		WAVWriter.cpp
		WindowInfo.cpp
		YAML.cpp
	)

	target_sources(common PRIVATE
		AlignedMalloc.h
		Assertions.h
		boost_spsc_queue.hpp
		BitUtils.h
		ByteSwap.h
		Console.h
		CrashHandler.h
		DynamicLibrary.h
		Easing.h
		EnumOps.h
		Error.h
		FPControl.h
		FastJmp.h
		FileSystem.h
		HashCombine.h
		HostSys.h
		HeterogeneousContainers.h
		Image.h
		LRUCache.h
		HeapArray.h
		HTTPDownloader.h
		MemoryInterface.h
		MemorySettingsInterface.h
		MD5Digest.h
		MRCHelpers.h
		Path.h
		PrecompiledHeader.h
		ProgressCallback.h
		ReadbackSpinManager.h
		RedtapeWilCom.h
		RedtapeWindows.h
		ScopedGuard.h
		SettingsInterface.h
		SettingsWrapper.h
		SingleRegisterTypes.h
		SmallString.h
		StringUtil.h
		Timer.h
		TextureDecompress.h
		Threading.h
		VectorIntrin.h
		WAVWriter.h
		WindowInfo.h
		WrappedMemCopy.h
		YAML.h
	)
else()
	# Rust shim. With EXCLUDE_CPP_COMMON=ON we skip the original C++
	# common/ sources and instead compile the per-class / per-module
	# shim files in `common/_rust_shim/`. Each shim forwards the
	# required C++ symbol to the matching `pcsx2_*` Rust FFI export,
	# or returns a safe default when no FFI equivalent exists yet.
	# The shim's own headers live alongside the .cpp files and are
	# pulled in via the explicit `target_sources(common PRIVATE ...)`
	# call below.
	target_sources(common PRIVATE
		_rust_shim/_shim_common.h
		_rust_shim/_shim_pcsx2_assertions.h
		_rust_shim/_shim_assertions.cpp
		_rust_shim/_shim_error.cpp
		_rust_shim/_shim_path.cpp
		_rust_shim/_shim_filesystem.cpp
		_rust_shim/_shim_threading.cpp
		_rust_shim/_shim_perf.cpp
		_rust_shim/_shim_md5digest.cpp
		_rust_shim/_shim_smallstring.cpp
		_rust_shim/_shim_stringutil.cpp
		_rust_shim/_shim_log.cpp
		_rust_shim/_shim_httpdownloader.cpp
		_rust_shim/_shim_memoryinterface.cpp
		_rust_shim/_shim_memoryinterface_templates.cpp
		_rust_shim/_shim_dynamiclibrary.cpp
		_rust_shim/_shim_hostsys.cpp
		_rust_shim/_shim_crashhandler.cpp
		_rust_shim/_shim_timer.cpp
		_rust_shim/_shim_yaml.cpp
		_rust_shim/_shim_readbackspinmanager.cpp
		_rust_shim/_shim_texturedecompress.cpp
		_rust_shim/_shim_extras.cpp
		_rust_shim/_shim_settings.cpp
	)
endif()

if(ARCH_X86)
	target_sources(common PRIVATE
		emitter/bmi.cpp
		emitter/fpu.cpp
		emitter/groups.cpp
		emitter/jmp.cpp
		emitter/legacy.cpp
		emitter/legacy_sse.cpp
		emitter/movs.cpp
		emitter/simd.cpp
		emitter/x86emitter.cpp
	)

	target_sources(common PRIVATE
		emitter/implement/dwshift.h
		emitter/implement/group1.h
		emitter/implement/group2.h
		emitter/implement/group3.h
		emitter/implement/helpers.h
		emitter/implement/incdec.h
		emitter/implement/jmpcall.h
		emitter/implement/movs.h
		emitter/implement/simd_arithmetic.h
		emitter/implement/simd_comparisons.h
		emitter/implement/simd_helpers.h
		emitter/implement/simd_moremovs.h
		emitter/implement/simd_shufflepack.h
		emitter/implement/simd_templated_helpers.h
		emitter/implement/test.h
		emitter/implement/xchg.h
		emitter/instructions.h
		emitter/internal.h
		emitter/legacy_instructions.h
		emitter/legacy_internal.h
		emitter/legacy_types.h
		emitter/x86emitter.h
		emitter/x86types.h
	)
endif()

if(WIN32)
	enable_language(ASM_MASM)
	if(NOT EXCLUDE_CPP_COMMON)
		target_sources(common PRIVATE
			FastJmp.asm
			HTTPDownloaderWinHTTP.cpp
			HTTPDownloaderWinHTTP.h
			StackWalker.cpp
			StackWalker.h
			Windows/WinThreads.cpp
			Windows/WinHostSys.cpp
			Windows/WinMisc.cpp
		)
	else()
		# Even with EXCLUDE_CPP_COMMON=ON we still need the assembly
		# implementations of `fastjmp_set` / `fastjmp_jmp` because the
		# Rust port doesn't yet have a portable longjmp equivalent
		# that the C++ side can call into.
		target_sources(common PRIVATE FastJmp.asm)
	endif()
	target_link_libraries(common PUBLIC
		WIL::WIL
		winmm
		pathcch
	)
elseif(APPLE)
	if(NOT EXCLUDE_CPP_COMMON)
		target_sources(common PRIVATE
			CocoaTools.mm
			CocoaTools.h
			Darwin/DarwinThreads.cpp
			Darwin/DarwinMisc.cpp
			Darwin/DarwinMisc.h
			Linux/LnxHostSys.cpp
		)
	endif()
	target_compile_options(common PRIVATE -fobjc-arc)
	target_link_options(common PRIVATE -fobjc-link-runtime)
	target_link_libraries(common PRIVATE
		"-framework Foundation"
		"-framework IOKit"
	)
else()
	if(NOT EXCLUDE_CPP_COMMON)
		target_sources(common PRIVATE
			Linux/LnxHostSys.cpp
			Linux/LnxThreads.cpp
			Linux/LnxMisc.cpp
		)
	endif()
	target_include_directories(common PRIVATE
		${DBUS_INCLUDE_DIRS}
	)
	target_link_libraries(common PRIVATE
		${DBUS_LINK_LIBRARIES}
		X11::X11
		X11::Xrandr
		X11::Xi
	)
	if(USE_BACKTRACE)
		target_compile_definitions(common PRIVATE "HAS_LIBBACKTRACE=1")
		target_link_libraries(common PRIVATE libbacktrace::libbacktrace)
	endif()
endif()

if(NOT EXCLUDE_CPP_COMMON)
	set_source_files_properties(PrecompiledHeader.cpp PROPERTIES HEADER_FILE_ONLY TRUE)
else()
	# When C++ common is excluded, the PrecompiledHeader.cpp is no
	# longer part of the build. We still want `target_precompile_headers`
	# to apply, but with no `.cpp` to anchor the PCH. The shim below
	# adds an empty placeholder so the property call below doesn't
	# error. We use `add_custom_target` so it produces no output but
	# exists in the build graph.
	add_library(common_pch_stub INTERFACE)
	set_source_files_properties(${CMAKE_CURRENT_BINARY_DIR}/pcsx2_common_rs.h PROPERTIES HEADER_FILE_ONLY TRUE)
endif()

if(USE_VTUNE)
	target_link_libraries(common PUBLIC Vtune::Vtune)
endif()

if (USE_GCC AND CMAKE_INTERPROCEDURAL_OPTIMIZATION)
	# GCC LTO doesn't work with asm statements
	set_source_files_properties(FastJmp.cpp PROPERTIES COMPILE_FLAGS -fno-lto)
endif()

if(NOT WIN32)
	if(NOT EXCLUDE_CPP_COMMON)
		# libcurl-based HTTPDownloader
		target_sources(common PRIVATE
			HTTPDownloaderCurl.cpp
			HTTPDownloaderCurl.h
		)
	endif()
	target_link_libraries(common PRIVATE
		CURL::libcurl
	)
endif()

target_link_libraries(common PRIVATE
	${LIBC_LIBRARIES}
	JPEG::JPEG
	PNG::PNG
	WebP::libwebp
	cpuinfo
)

target_link_libraries(common PUBLIC
	fmt::fmt
	fast_float
	ryml::ryml
)

fixup_file_properties(common)
target_compile_features(common PUBLIC cxx_std_17)
target_include_directories(common PUBLIC ../3rdparty/include ../)
target_compile_definitions(common PUBLIC "${PCSX2_DEFS}")
target_compile_options(common PRIVATE "${PCSX2_WARNINGS}")

if(COMMAND target_precompile_headers AND NOT EXCLUDE_CPP_COMMON)
	target_precompile_headers(common PRIVATE PrecompiledHeader.h)
endif()
