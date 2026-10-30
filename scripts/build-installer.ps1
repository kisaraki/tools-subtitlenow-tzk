param([switch]$SkipBuild, [string]$Compiler)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    $version = [regex]::Match((Get-Content Cargo.toml -Raw), '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
    if (-not $version) { throw 'Cargo.toml must contain a stable numeric version.' }
    if (-not $SkipBuild) {
        if ($env:CARGO_ENCODED_RUSTFLAGS) { throw 'Unset CARGO_ENCODED_RUSTFLAGS so the installer build can enforce static CRT linkage.' }
        $previousRustFlags = $env:RUSTFLAGS
        try {
            $env:RUSTFLAGS = "$previousRustFlags -C target-feature=+crt-static".Trim()
            # An explicit target keeps the CRT flag off host proc-macro libraries.
            cargo build --locked --release --target x86_64-pc-windows-msvc --bins
            if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
        } finally { $env:RUSTFLAGS = $previousRustFlags }
    }
    & (Join-Path $PSScriptRoot 'check-runtime-dependencies.ps1')
    if (-not $Compiler) {
        $candidates = @(
            "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
            "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
        )
        $Compiler = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    }
    if (-not $Compiler) { throw 'Install Inno Setup: winget install --id JRSoftware.InnoSetup --exact' }
    & (Join-Path $PSScriptRoot 'collect-licenses.ps1')
    $outputDir = Join-Path $projectRoot 'dist'
    & $Compiler "/DAppVersion=$version" "/DSourceDir=$projectRoot" "/DOutputDir=$outputDir" 'installer/subtitlenow.iss'
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath "$outputDir/setup.exe").Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$outputDir/SHA256SUMS.txt", "$hash  setup.exe`n", [Text.UTF8Encoding]::new($false))
    Write-Output "Built $outputDir/setup.exe ($version)"
} finally { Pop-Location }
