param([string]$BinaryDir)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
if (-not $BinaryDir) { $BinaryDir = Join-Path $projectRoot 'target\x86_64-pc-windows-msvc\release' }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Visual Studio C++ Build Tools are required for dependency verification.' }
$dumpbin = & $vswhere -latest -products '*' -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe' | Select-Object -First 1
if (-not $dumpbin) { throw 'Cannot find dumpbin.exe in Visual Studio C++ Build Tools.' }
foreach ($name in @('subtitlenow-desktop.exe', 'subtitlenow.exe')) {
    $binary = Join-Path $BinaryDir $name
    if (-not (Test-Path -LiteralPath $binary)) { throw "Missing release binary: $name" }
    $imports = & $dumpbin /dependents $binary | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Cannot inspect release dependencies: $name" }
    if ($imports -match '(?im)^\s*(?:vcruntime\d[^\s]*|msvcp\d[^\s]*|msvcr\d[^\s]*)\.dll\s*$') {
        throw "$name depends on a separately installed Visual C++ runtime; rebuild with static CRT."
    }
    Write-Output "PASS: $name has no external Visual C++ runtime DLL dependency."
}
