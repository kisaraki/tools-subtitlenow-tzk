param(
    [string]$Compiler = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    [string]$OldVersion = '0.1.1',
    [string]$OldBinaryDir
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$newVersion = [regex]::Match((Get-Content (Join-Path $projectRoot 'Cargo.toml') -Raw), '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
if (-not $newVersion -or [version]$OldVersion -ge [version]$newVersion) { throw 'Validation requires an older fixture version.' }
$binaryDir = Join-Path $projectRoot 'target\x86_64-pc-windows-msvc\release'
if (-not $OldBinaryDir) { $OldBinaryDir = $binaryDir }
$testId = 'SubtitleNow.InstallerValidation'
$key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${testId}_is1"
if (Test-Path -LiteralPath $key) { throw 'A prior validation installation exists; inspect it before retrying.' }
$testRoot = Join-Path $projectRoot ('artifacts\installer-' + [Guid]::NewGuid().ToString('N'))
$installDir = Join-Path $testRoot 'installed'
New-Item -ItemType Directory -Path $testRoot | Out-Null
$source = Get-Content -LiteralPath (Join-Path $projectRoot 'installer\subtitlenow.iss') -Raw
$source = $source.Replace('MessagesFile: "ChineseTraditional.isl"', 'MessagesFile: "' + $projectRoot + '\installer\ChineseTraditional.isl"')
$marker = Join-Path $testRoot 'obsolete.txt'
[IO.File]::WriteAllText($marker, 'Old-version-only validation fixture')
$oldSource = $source.Replace('[Files]', '[Files]' + "`n" + 'Source: "' + $marker + '"; DestDir: "{app}"')
$oldScript = Join-Path $testRoot 'old.iss'
$newScript = Join-Path $testRoot 'new.iss'
[IO.File]::WriteAllText($oldScript, $oldSource, [Text.UTF8Encoding]::new($true))
[IO.File]::WriteAllText($newScript, $source, [Text.UTF8Encoding]::new($true))
foreach ($item in @(@('old',$OldVersion,$oldScript,$OldBinaryDir), @('new',$newVersion,$newScript,$binaryDir))) {
    & $Compiler "/DAppVersion=$($item[1])" "/DProductId=$testId" '/DProductName=SubtitleNow Installer Validation' "/DSourceDir=$projectRoot" "/DBinaryDir=$($item[3])" "/DOutputDir=$testRoot\$($item[0])" $item[2] *> "$testRoot\compile-$($item[0]).log"
    if ($LASTEXITCODE -ne 0) { throw "Compiler failed: $testRoot\compile-$($item[0]).log" }
}
function Invoke-TestSetup([string]$Name) {
    $setupPath = Join-Path $testRoot "$Name\setup.exe"
    $process = Start-Process -FilePath $setupPath -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',"/DIR=`"$installDir`"", "/LOG=`"$testRoot\$Name-install.log`"") -WindowStyle Hidden -PassThru -Wait
    return $process.ExitCode
}
try {
    if ((Invoke-TestSetup 'old') -ne 0) { throw 'Old fixture install failed.' }
    if ((Get-ItemProperty -LiteralPath $key).DisplayVersion -ne $OldVersion) { throw 'Old version registration mismatch.' }
    if (-not (Test-Path -LiteralPath "$installDir\obsolete.txt")) { throw 'Old fixture not installed.' }
    if ((Invoke-TestSetup 'new') -ne 0) { throw 'Upgrade failed.' }
    if ((Get-ItemProperty -LiteralPath $key).DisplayVersion -ne $newVersion) { throw 'New version registration mismatch.' }
    if (Test-Path -LiteralPath "$installDir\obsolete.txt") { throw 'Old-only file survived; uninstall did not run.' }
    if (-not (Select-String -LiteralPath "$testRoot\new-install.log" -SimpleMatch 'Previous version removed successfully')) { throw 'Missing uninstall completion evidence.' }
    $actual = (Get-FileHash -LiteralPath "$installDir\subtitlenow-desktop.exe").Hash
    $expected = (Get-FileHash -LiteralPath "$binaryDir\subtitlenow-desktop.exe").Hash
    if ($actual -ne $expected) { throw 'Installed binary differs from release.' }
    $smoke = Start-Process -FilePath "$installDir\subtitlenow.exe" -ArgumentList '--help' -WindowStyle Hidden -PassThru -RedirectStandardOutput "$testRoot\help.txt" -RedirectStandardError "$testRoot\help-error.txt"
    if (-not $smoke.WaitForExit(10000)) { $smoke.Kill(); throw 'Installed executable startup timed out.' }
    if ($smoke.ExitCode -ne 0 -or (Get-Item -LiteralPath "$testRoot\help.txt").Length -eq 0) { throw 'Installed executable failed to start.' }
    if ((Invoke-TestSetup 'old') -eq 0) { throw 'Downgrade was incorrectly accepted.' }
    if ((Get-ItemProperty -LiteralPath $key).DisplayVersion -ne $newVersion) { throw 'Downgrade changed registration.' }
    Write-Output 'PASS: fresh install, previous uninstall, obsolete-file removal, binary hash, executable startup, downgrade rejection.'
} finally {
    if (Test-Path -LiteralPath $key) {
        $registered = (Get-ItemProperty -LiteralPath $key).InstallLocation.TrimEnd('\')
        if ([IO.Path]::GetFullPath($registered) -ne [IO.Path]::GetFullPath($installDir)) { throw 'Refusing cleanup outside isolated test installation.' }
        $uninstaller = (Get-ItemProperty -LiteralPath $key).UninstallString.Trim('"')
        if ([IO.Path]::GetDirectoryName($uninstaller) -ne $registered) { throw 'Unexpected test uninstaller path.' }
        $process = Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -WindowStyle Hidden -PassThru -Wait
        if ($process.ExitCode -ne 0) { throw 'Validation uninstall failed.' }
    }
}
if ((Test-Path -LiteralPath $key) -or (Test-Path -LiteralPath "$installDir\subtitlenow-desktop.exe")) { throw 'Test cleanup did not remove registration or binary.' }
Write-Output "PASS: final uninstall. Logs: $testRoot"
