# Native/cross MSVC build. WiX 5.0.2 packages all payload and target notices.
param(
    [Parameter(Mandatory=$true)][string]$Version,
    [Parameter(Mandatory=$true)][ValidateSet('x64','x86','arm64')][string]$Arch
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($Version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw 'Invalid stable SemVer' }
$Parts = $Version.Split('.')
if ([long]$Parts[0] -gt 255 -or [long]$Parts[1] -gt 255 -or [long]$Parts[2] -gt 65535) { throw 'Version exceeds Windows MSI version limits (255.255.65535)' }
if (-not $IsWindows) { throw 'A Windows build host is required' }
$Root = Split-Path -Parent $PSScriptRoot
$Target = @{ x64='x86_64-pc-windows-msvc'; x86='i686-pc-windows-msvc'; arm64='aarch64-pc-windows-msvc' }[$Arch]
$UpgradeCode = @{ x64='C2645B06-62B5-487E-B447-F843BB1E5082'; x86='9649C190-AF65-467E-ABEA-8B570B5CE13A'; arm64='4A01FA6D-AD31-46AA-9935-55A1A21BC16D' }[$Arch]
function Invoke-Checked {
    param([string]$Tool, [string[]]$Arguments)
    & $Tool @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Tool failed with exit code $LASTEXITCODE" }
}
foreach ($Tool in @('cargo','rustc','python','wix')) { Get-Command $Tool -ErrorAction Stop | Out-Null }
$Stage = Join-Path ([System.IO.Path]::GetTempPath()) ('nntpbrowser-package-' + [guid]::NewGuid())
$OriginalFlags = $env:RUSTFLAGS
$OriginalTargetFlags = $env:CARGO_ENCODED_RUSTFLAGS
Push-Location $Root
try {
    Invoke-Checked python @('scripts/version.py','set',$Version)
    # Bundle the CRT statically so portable packages do not require a separate
    # Visual C++ Redistributable installer on the user's machine.
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
    Invoke-Checked cargo @('build','--locked','--release','--target',$Target,'-p','app','--bin','nntpbrowser','--bin','nntpbrowser-cli')
    New-Item -ItemType Directory -Force -Path $Stage | Out-Null
    $Metadata = Join-Path $Stage 'metadata.json'
    $Json = & cargo metadata --locked --format-version 1 --filter-platform $Target
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
    [System.IO.File]::WriteAllText($Metadata, ($Json -join "`n"), [System.Text.UTF8Encoding]::new($false))
    $TargetDirectory = (($Json -join "`n") | ConvertFrom-Json).target_directory
    $Payload = Join-Path $Stage "nntpbrowser_${Version}_windows_${Arch}"
    $Docs = Join-Path $Payload 'share/doc/nntpbrowser'
    New-Item -ItemType Directory -Force -Path $Docs | Out-Null
    Copy-Item "$TargetDirectory/$Target/release/nntpbrowser.exe" (Join-Path $Payload 'nntpbrowser.exe')
    Copy-Item "$TargetDirectory/$Target/release/nntpbrowser-cli.exe" (Join-Path $Payload 'nntpbrowser-cli.exe')
    Copy-Item LICENSE, docs/dependencies.md, packaging/RUNTIME.md $Docs
    Invoke-Checked python @('scripts/collect-notices.py',$Metadata,(Join-Path $Docs 'third-party'))
    $Dist = Join-Path $Root 'dist'
    New-Item -ItemType Directory -Force -Path $Dist | Out-Null
    $Msi = Join-Path $Stage "nntpbrowser_${Version}_windows_${Arch}.msi"
    Invoke-Checked wix @('build','packaging/windows/installer.wxs','-arch',$Arch,
        '-d',"Version=$Version",'-d',"Arch=$Arch",'-d',"UpgradeCode=$UpgradeCode",
        '-d',"Stage=$Payload",'-d',"Icon=$Root/packaging/windows/app-icon.ico",'-o',$Msi)
    $Zip = Join-Path $Stage "nntpbrowser_${Version}_windows_${Arch}_portable.zip"
    # zipfile includes hidden upstream metadata/source files, unlike the
    # PowerShell Compress-Archive cmdlet's hidden-file omission.
    $ZipCode = 'import pathlib,sys,zipfile; p=pathlib.Path(sys.argv[1]); z=zipfile.ZipFile(sys.argv[2],"w",compression=zipfile.ZIP_DEFLATED,strict_timestamps=False); [z.write(f,f.relative_to(p.parent).as_posix()) for f in sorted(p.rglob("*")) if f.is_file()]; z.close()'
    Invoke-Checked python @('-c',$ZipCode,$Payload,$Zip)
    # Administrative extraction exercises the generated MSI's real file table
    # and cabinets without installing the application on the build runner.
    $Extracted = Join-Path $Stage 'msi-extracted'
    $MsiLog = Join-Path $Stage 'msi-extract.log'
    $Installer = Start-Process msiexec.exe -ArgumentList @('/a', ('"' + $Msi + '"'),
        '/qn', ('TARGETDIR="' + $Extracted + '"'), '/l*v', ('"' + $MsiLog + '"')) -Wait -PassThru
    if ($Installer.ExitCode -ne 0) {
        if (Test-Path $MsiLog) { Get-Content $MsiLog | Write-Output }
        throw "MSI administrative extraction failed: $($Installer.ExitCode)"
    }
    $InstalledExecutables = @(Get-ChildItem -LiteralPath $Extracted -Filter 'nntpbrowser.exe' -File -Recurse)
    if ($InstalledExecutables.Count -ne 1) { throw 'MSI must contain exactly one GUI executable' }
    $InstalledPayload = $InstalledExecutables[0].DirectoryName
    foreach ($Source in Get-ChildItem -LiteralPath $Payload -File -Recurse -Force) {
        $Relative = [System.IO.Path]::GetRelativePath($Payload, $Source.FullName)
        $InstalledFile = Join-Path $InstalledPayload $Relative
        if (-not (Test-Path -LiteralPath $InstalledFile -PathType Leaf)) { throw "MSI omits payload file: $Relative" }
        if ((Get-FileHash -LiteralPath $Source.FullName).Hash -ne (Get-FileHash -LiteralPath $InstalledFile).Hash) {
            throw "MSI payload differs from source: $Relative"
        }
    }
    $ZipCheck = 'import pathlib,sys,zipfile; p=pathlib.Path(sys.argv[1]); z=zipfile.ZipFile(sys.argv[2]); files={f.relative_to(p.parent).as_posix():f for f in p.rglob("*") if f.is_file()}; assert set(z.namelist())==set(files); assert all(z.read(n)==f.read_bytes() for n,f in files.items()); assert z.testzip() is None'
    Invoke-Checked python @('-c',$ZipCheck,$Payload,$Zip)
    # Run native targets; ARM64 cross builds are exercised on the ARM runner.
    if ($Arch -ne 'arm64' -or $env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
        Invoke-Checked (Join-Path $Payload 'nntpbrowser-cli.exe') @('--version')
    }
    Move-Item -Force $Msi,$Zip $Dist
    Write-Output "Built Windows $Arch MSI and portable ZIP in dist/"
} finally {
    Pop-Location
    $env:RUSTFLAGS = $OriginalFlags
    if ($null -eq $OriginalTargetFlags) { Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue }
    else { $env:CARGO_ENCODED_RUSTFLAGS = $OriginalTargetFlags }
    if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
}
