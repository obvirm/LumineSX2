Push-Location 'E:\project\pcsx2'
$id = (Get-Content 'push2_pid.txt' -ErrorAction SilentlyContinue)
$p = Get-Process -Id $id -ErrorAction SilentlyContinue
if ($p) { Write-Output ("PID " + $id + " ALIVE elapsed=" + [math]::Round(((Get-Date)-$p.StartTime).TotalMinutes,1) + "min") } else { Write-Output ("PID " + $id + " EXITED") }
Write-Output "=== push2_err.log tail ==="
Get-Content 'push2_err.log' -ErrorAction SilentlyContinue | Select-Object -Last 8
Write-Output "=== remote refs ==="
git ls-remote origin 2>&1 | Select-Object -First 2
Pop-Location
