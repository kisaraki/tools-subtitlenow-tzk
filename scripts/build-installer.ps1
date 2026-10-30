param([switch]$SkipBuild, [string]$Compiler)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    $version = [regex]::Match((Get-Content Cargo.toml -Raw), '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
    if (-not $version) { throw 'Cargo.toml must contain a stable numeric version.' }
    if (-not $SkipBuild) {
        cargo build --locked --release --bins
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
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
