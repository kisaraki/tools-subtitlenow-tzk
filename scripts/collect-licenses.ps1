$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    $metadata = (cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc | Out-String) | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve package metadata.' }
    $resolved = @{}
    foreach ($node in $metadata.resolve.nodes) { $resolved[$node.id] = $true }
    $outputDir = Join-Path $projectRoot 'target\package-licenses'
    New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
    $index = [Collections.Generic.List[string]]::new()
    $index.Add('Third-party packages resolved for Windows x64; versions fixed in Cargo.lock.')
    foreach ($package in $metadata.packages) {
        if (-not $package.source -or -not $resolved.ContainsKey($package.id)) { continue }
        $packageRoot = Split-Path $package.manifest_path
        $destination = Join-Path $outputDir "$($package.name)-$($package.version)"
        New-Item -ItemType Directory -Path $destination -Force | Out-Null
        $files = Get-ChildItem -LiteralPath $packageRoot | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' }
        foreach ($file in $files) { Copy-Item -LiteralPath $file.FullName -Destination $destination -Recurse -Force }
        $index.Add("$($package.name) $($package.version): $($package.license); $($package.repository)")
        if (-not $files) {
            # Existing upstream notices and provenance are kept in licenses/.
            if ($package.name -notin @('realfft','rusty-chromaprint','whatlang')) { throw "Missing license notice for $($package.name)." }
            Copy-Item -LiteralPath $package.manifest_path -Destination "$destination\Cargo.toml"
            Copy-Item -LiteralPath "$packageRoot\README.md" -Destination "$destination\README.md"
        }
    }
    [IO.File]::WriteAllLines("$outputDir\PACKAGES.txt", $index, [Text.UTF8Encoding]::new($false))
} finally { Pop-Location }
