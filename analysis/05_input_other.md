# Analysis: Input, DEV9, USB, TextureReplacement — External Library Calls

## 1. Input/SDLInputSource.h & .cpp

**External dependency: SDL3 (`<SDL3/SDL.h>`)**

SDL3 API calls used:
| Function | Purpose |
|----------|---------|
| `SDL_InitSubSystem(SDL_INIT_JOYSTICK \| SDL_INIT_GAMEPAD \| SDL_INIT_HAPTIC)` | Init SDL subsystems |
| `SDL_QuitSubSystem(...)` | Shutdown SDL |
| `SDL_PollEvent(&ev)` | Poll input events |
| `SDL_OpenGamepad(index)` / `SDL_CloseGamepad(gamepad)` | Gamepad open/close |
| `SDL_OpenJoystick(index)` / `SDL_CloseJoystick(joystick)` | Joystick open/close |
| `SDL_GetGamepadJoystick(gamepad)` | Get joystick from gamepad |
| `SDL_GetJoystickID(joystick)` | Get joystick ID |
| `SDL_GetGamepadName(gamepad)` / `SDL_GetJoystickName(joystick)` | Device name |
| `SDL_GetGamepadPlayerIndex(gamepad)` | Player index |
| `SDL_GetGamepadBindings(gamepad, &count)` | Button bindings |
| `SDL_GetGamepadMappings(&count)` | Controller mappings |
| `SDL_GetGamepadProperties(gamepad)` | Properties |
| `SDL_GetGamepadButtonLabel(gamepad, btn)` | Button label |
| `SDL_RumbleGamepad(gamepad, ...)` / `SDL_RumbleJoystick(...)` | Force feedback |
| `SDL_SetGamepadLED(gamepad, r, g, b)` | LED color |
| `SDL_HapticRumbleSupported(haptic)` | Haptic check |
| `SDL_InitHapticRumble(haptic)` | Haptic init |
| `SDL_GetError()` | Error retrieval |

**Rust replacement**: `gilrs` crate (gamepad input), `winit` (event loop)

---

## 2. Input/DInputSource.h & .cpp

**External dependency: DirectInput 8 (`<dinput.h>`), WIL (`<wil/resource.h>`)**

DirectInput API calls:
| Function | Purpose |
|----------|---------|
| `DirectInput8Create(hinst, version, iid, &out, null)` | Create DirectInput object |
| `IDirectInput8W::EnumDevices(DI8DEVCLASS_GAMECTRL, callback, &data, DIEDFL_ATTACHEDONLY)` | Enumerate controllers |
| `IDirectInput8W::CreateDevice(guid, &device, null)` | Create device |
| `IDirectInputDevice8W::SetCooperativeLevel(hwnd, DISCL_BACKGROUND \| DISCL_EXCLUSIVE)` | Set coop level |
| `IDirectInputDevice8W::SetDataFormat(&c_dfDIJoystick2)` | Set data format |
| `IDirectInputDevice8W::Acquire()` / `Unacquire()` | Device acquisition |
| `IDirectInputDevice8W::Poll()` | Poll device |
| `IDirectInputDevice8W::GetDeviceState(size, &state)` | Read device state |

WIL types: `wil::com_ptr_nothrow<IDirectInput8W>`, `wil::unique_hmodule`

**Rust replacement**: `gilrs` crate (cross-platform), on Windows can also use `windows` crate with `DirectInput`

---

## 3. DEV9/Win32/tap-win32.cpp

**External dependencies: Win32 API (iphlpapi, ws2_32, setupapi, com), fmt**

| API | Purpose |
|-----|---------|
| `CreateFileA(...)` | Open TAP driver device |
| `DeviceIoControl(...)` | TAP IOCTL: GET_MAC, SET_MEDIA_STATUS, GET_VERSION |
| `CloseHandle(...)` | Close handles |
| SetupDi functions | Device enumeration |
| `INetCfg*` COM interfaces | Network configuration |
| `GetAdaptersAddresses()` etc (iphlpapi) | Adapter info |
| `WSAStartup()` etc (ws2_32) | Windows sockets |
| `CoInitializeEx()` | COM init |
| `fmt::format(...)` | String formatting |

**Rust replacement**: `windows` crate + `tokio` for async. DEV9 (network adapter emulation for PS2 HDD/network) is optional — skip for initial port.

---

## 4. DEV9/ATA/HddCreate.cpp

**External dependencies: fmt (`<fmt/format.h>`), Windows `<winioctl.h>`, `<io.h>`**

| API | Purpose |
|-----|---------|
| `FileSystem::FileExists()` | Internal PCSX2 common |
| `FileSystem::OpenManagedCFile()` | Internal PCSX2 common |
| `SetFileValidData()` / `SetFilePointerEx()` / `SetEndOfFile()` | Sparse file (Windows) |
| `fmt::format(...)` | String formatting |

**Rust replacement**: `std::fs` (built-in) for file operations. Minimal external deps.

---

## 5. USB/usb-mic/audiodev-cubeb.cpp

**External dependencies: cubeb (`<cubeb/cubeb.h>`), fmt, WIL (`<wil/resource.h>`), COM**

| API | Purpose |
|-----|---------|
| `cubeb_init(&ctx, "PCSX2_USB", backend)` | Initialize cubeb audio |
| `cubeb_destroy(ctx)` | Destroy cubeb context |
| `cubeb_enumerate_devices(ctx, type, &collection)` | Enumerate audio devices |
| `cubeb_device_collection_destroy(ctx, &collection)` | Free device list |
| `cubeb_stream_init(ctx, &stream, name, params, ...)` | Create audio stream |
| `cubeb_stream_start(stream)` / `cubeb_stream_stop(stream)` | Start/stop stream |
| `cubeb_stream_destroy(stream)` | Destroy stream |
| `cubeb_stream_get_position(stream)` | Get current position |
| `CoInitializeEx(...)` / `CoUninitialize()` | COM init (Windows) |
| `fmt::format(...)` | String formatting |

**Rust replacement**: `cpal` crate (cross-platform audio). Simpler API, Rust-native.

---

## 6. GS/Renderers/HW/GSTextureReplacementLoaders.cpp

**External dependency: libpng (`<png.h>`)**

| API | Purpose |
|-----|---------|
| `png_create_read_struct(PNG_LIBPNG_VER_STRING, ...)` | Create PNG reader |
| `png_create_info_struct(png_ptr)` | Create info struct |
| `png_destroy_read_struct(&png_ptr, &info_ptr, null)` | Destroy reader |
| `png_init_io(png_ptr, fp)` | Set I/O |
| `png_read_info(png_ptr, info_ptr)` | Read PNG header |
| `png_get_IHDR(...)` | Get image dimensions |
| `png_get_rowbytes(png_ptr, info_ptr)` | Get row size |
| `png_read_row(png_ptr, row, null)` | Read row |
| `png_create_write_struct(...)` | Create PNG writer |
| `png_create_info_struct(...)` | Create write info |
| `png_set_IHDR(...)` | Set header |
| `png_write_info(png_ptr, info_ptr)` | Write header |
| `png_write_row(png_ptr, row)` | Write row |
| `png_write_end(png_ptr, info_ptr)` | Finalize |

**Rust replacement**: `image` crate or `png` crate. Both are pure Rust.

---

## 7. common/Image.cpp (bonus — linked via common.lib)

**External dependencies: libjpeg (`<jpeglib.h>`), libpng (`<png.h>`), libwebp (`<webp/decode.h>`, `<webp/encode.h>`)**

| API | Purpose |
|-----|---------|
| `jpeg_create_decompress(&info)` | JPEG decompress |
| `jpeg_read_header(&info, TRUE)` | Read JPEG header |
| `jpeg_start_decompress(&info)` | Start decompression |
| `jpeg_read_scanlines(&info, buffer, count)` | Read scanlines |
| `jpeg_finish_decompress(&info)` | Finish |
| `jpeg_destroy_decompress(&info)` | Cleanup |
| `png_create_read_struct(...)` etc | PNG read |
| `WebPGetInfo(data, size, &w, &h)` | WebP decode |
| `WebPDecodeRGBA(data, size, &w, &h)` | WebP -> RGBA |
| `WebPEncodeRGBA(rgba, w, h, stride, quality, &out, &size)` | WebP encode |

**Rust replacement**: `image` crate covers PNG, JPEG, WebP.

---

## Summary Table

| File | External Libs | Complexity | Rust Replacement |
|------|---------------|------------|------------------|
| `Input/SDLInputSource.cpp` | SDL3 | **High** (500+ lines, many SDL APIs) | `gilrs` + `winit` |
| `Input/DInputSource.cpp` | DirectInput, WIL | **Medium** (Win32 only) | `gilrs` (skip, cross-platform) |
| `DEV9/Win32/tap-win32.cpp` | Win32 (iphlpapi, setupapi, COM), fmt | **High** (600+ lines, complex) | Skip (optional feature) |
| `DEV9/ATA/HddCreate.cpp` | fmt, Windows file API | **Low** | `std::fs` |
| `USB/usb-mic/audiodev-cubeb.cpp` | cubeb, COM, WIL, fmt | **Medium** | `cpal` |
| `GSTextureReplacementLoaders.cpp` | libpng | **Medium** (800+ lines) | `png` / `image` crate |
| `common/Image.cpp` | libjpeg, libpng, libwebp | **Medium** | `image` crate |
