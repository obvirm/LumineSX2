@echo off
set "VCToolsInstallDir=C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Tools\MSVC\14.51.36231"
set "WindowsSdkDir=C:\Program Files (x86)\Windows Kits\10"
set "WindowsSDKVersion=10.0.26100.0"

set "PATH=%VCToolsInstallDir%\bin\Hostx64\x64;C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64;%PATH%"

set "INCLUDE=%VCToolsInstallDir%\include;%VCToolsInstallDir%\atlmfc\include;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\ucrt;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\shared;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\winrt"
set "LIB=%VCToolsInstallDir%\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64"

cd /d E:\project\pcsx2\build
"C:\Users\X\vcpkg\downloads\tools\cmake-4.3.2-windows\cmake-4.3.2-windows-x86_64\bin\cmake.exe" -G "NMake Makefiles" -DCMAKE_BUILD_TYPE=Release -DENABLE_QT_UI=OFF -DENABLE_TESTS=OFF -DENABLE_GSRUNNER=ON -DUSE_RUST_COMMON=ON -DCMAKE_DISABLE_FIND_PACKAGE_Vtune=TRUE -DCMAKE_PREFIX_PATH=E:/project/pcsx2/build -DCMAKE_TOOLCHAIN_FILE=C:/Users/X/vcpkg/scripts/buildsystems/vcpkg.cmake -DVCPKG_TARGET_TRIPLET=x64-windows .. 2>&1