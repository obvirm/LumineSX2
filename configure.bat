@echo off
REM Set MSVC env vars manually (vcvars64 requires vswhich.exe which isn't installed)
set "VCINSTALLDIR=C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC"
set "VCToolsInstallDir=C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Tools\MSVC\14.51.36231"
set "WindowsSdkDir=C:\Program Files (x86)\Windows Kits\10"
set "WindowsSDKVersion=10.0.26100.0"

set "PATH=%VCToolsInstallDir%\bin\Hostx64\x64;C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64;C:\Program Files (x86)\Windows Kits\10\bin\x64;%PATH%"

set "INCLUDE=%VCToolsInstallDir%\include;%VCToolsInstallDir%\atlmfc\include;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\ucrt;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\shared;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\winrt"
set "LIB=%VCToolsInstallDir%\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64"

cd /d E:\project\pcsx2\rust\demo
call cl /EHsc /std:c++17 /O2 demo_test.cpp ..\common\target\release\pcsx2_common_rs.lib kernel32.lib ws2_32.lib advapi32.lib crypt32.lib userenv.lib bcrypt.lib ntdll.lib /OUT:demo_test.exe 2>&1