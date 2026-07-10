Push-Location 'E:\project\pcsx2'

# Kill stuck push
$id = Get-Content 'push2_pid.txt' -ErrorAction SilentlyContinue
if ($id) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
Get-Process git -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 3

Write-Output "=== Creating orphan branch lumine-sx2 ==="
# Stash any uncommitted changes
git stash --include-untracked 2>&1 | Out-Null

# Create orphan branch
git checkout --orphan lumine-sx2 2>&1 | Out-String
git rm -rf --cached . 2>&1 | Out-Null

# Re-add everything (will skip things in .gitignore)
git add -A 2>&1 | Out-Null

# Commit
$commitMsg = "LumineSX2 v0.1.0-alpha: Slint UI + Rust core rewrite (PCSX2 fork)

- Rust-based CDVD reader (ISO/CHD/CSO) with C FFI bridge
- pcsx2_common_rs: Rust common utilities (StringUtil, SettingsWrapper, threading)
- Slint UI: Material You dark theme, Android-style bottom nav
- Experimental warning: Windows-only, Android not yet supported
- Vulkan renderer for Android compatibility
- PCSX2 emulation core (C++) linked via static lib"

git commit -m $commitMsg 2>&1 | Select-Object -Last 5
Write-Output "=== orphan commit done ==="
Pop-Location
