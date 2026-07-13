@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64
cd /d E:\project\pcsx2\build

REM Always force a clean compile. nmake's "up-to-date" check is unreliable here:
REM editing pcsx2_capi.cpp/.h can leave the .obj stale and the linker then fails
REM with LNK2019 "unresolved external symbol pcsx2_* / DebugInterface_*".
del /q pcsx2\CMakeFiles\pcsx2_capi.dir\pcsx2_capi.cpp.obj 2>nul
del /q pcsx2\CMakeFiles\pcsx2_capi.dir\cmake_pch.cxx.obj 2>nul
del /q capi\pcsx2_capi.lib 2>nul

nmake -f pcsx2\CMakeFiles\pcsx2_capi.dir\build.make pcsx2\CMakeFiles\pcsx2_capi.dir\pcsx2_capi.cpp.obj
if errorlevel 1 goto :fail
nmake -f pcsx2\CMakeFiles\pcsx2_capi.dir\build.make pcsx2\CMakeFiles\pcsx2_capi.dir\cmake_pch.cxx.obj
if errorlevel 1 goto :fail
lib /nologo /out:capi\pcsx2_capi.lib pcsx2\CMakeFiles\pcsx2_capi.dir\pcsx2_capi.cpp.obj pcsx2\CMakeFiles\pcsx2_capi.dir\cmake_pch.cxx.obj
if errorlevel 1 goto :fail

REM Sanity: confirm the required entry points are present in the fresh archive.
dumpbin /symbols capi\pcsx2_capi.lib | findstr /C:"| pcsx2_initialize" /C:"| DebugInterface_getRegister" /C:"| CBreakPoints_AddBreakPoint" >nul
if errorlevel 1 (
  echo SYMBOL CHECK FAILED
  goto :fail
)

echo DONE
goto :end
:fail
echo COMPILE FAILED
:end
