# Builds the installer for Windows: target\installer\opensecagent-<version>-x64-setup.exe
#
# Builds opensecagent and opensecagent-enroll (the start-menu entry) in release mode with the version from
# Cargo.toml, checks the list of third-party licenses and calls the compiler of Inno Setup 6
# (packaging\windows\opensecagent.iss). Only on request (CLAUDE.md); raise the
# version in Cargo.toml first, as for the .deb.
#
# Needs: Rust with the MSVC tool chain, Python 3 and Inno Setup 6 (winget install JRSoftware.InnoSetup).
# Usage, from any folder:
#     powershell -ExecutionPolicy Bypass -File dev\build-installer.ps1
#
# Signing (docs/windows.md, section 6): with the thumbprint (SHA-1) of the code signing certificate, given as
# -CertSha1 or in the variable OPENSECAGENT_SIGN_SHA1, it signs both programs, the installer, the uninstaller and
# the temporary copies Setup starts with signtool of the Windows SDK, SHA-256 and a time stamp, and checks the
# signatures. The certificate must be in the certificate store of the user; with Certum in the cloud, SimplySign
# Desktop has to be connected. Without a thumbprint the installer stays unsigned, and the script says so.
#     powershell -ExecutionPolicy Bypass -File dev\build-installer.ps1 -CertSha1 <thumbprint>
#
# The own Ed25519 signature for updates through opensecmon (.sig) comes afterwards, on the computer with the key of
# the publisher, because Authenticode changes the file.

param(
    [string]$CertSha1 = $env:OPENSECAGENT_SIGN_SHA1,
    # the time stamp service of Certum, as in its instructions for signtool
    [string]$TimestampUrl = 'http://time.certum.pl'
)

# not 'Stop': PowerShell 5.1 would take the progress cargo writes to stderr for errors; every step checks its exit code
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# the first `version = "..."` is the one of [package] (ASCII only: PowerShell 5.1 reads this file as ANSI)
$line = Select-String -Path 'Cargo.toml' -Pattern '^version = "([^"]+)"' | Select-Object -First 1
if (-not $line) { throw 'no version in Cargo.toml' }
$version = $line.Matches[0].Groups[1].Value

$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (($env:PATH -split ';') -notcontains $cargoBin) { $env:PATH = "$cargoBin;$env:PATH" }

$iscc = @(
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
    (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 is missing: winget install JRSoftware.InnoSetup' }

# signtool of the Windows SDK (comes with Visual Studio, or: winget install Microsoft.WindowsSDK.10.0.26100)
function Find-SignTool {
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    if (-not (Test-Path $kits)) { return $null }
    Get-ChildItem -Path $kits -Directory |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
        Where-Object { Test-Path $_ } |
        Select-Object -First 1
}

$sign = [bool]$CertSha1
if ($sign) {
    $CertSha1 = ($CertSha1 -replace '[^0-9A-Fa-f]', '').ToUpper()
    if ($CertSha1.Length -ne 40) { throw 'the thumbprint of the certificate has 40 hexadecimal digits' }
    $signtool = Find-SignTool
    if (-not $signtool) { throw 'signtool is missing: install the Windows SDK (winget install Microsoft.WindowsSDK.10.0.26100)' }
    $cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Thumbprint -eq $CertSha1 }
    if (-not $cert) { throw "no certificate $CertSha1 in the store of the user (is SimplySign Desktop connected?)" }
    if ($cert.NotAfter -lt (Get-Date)) { throw "the certificate $CertSha1 expired on $($cert.NotAfter)" }
    Write-Host "Signing with: $($cert.Subject), valid until $($cert.NotAfter.ToString('yyyy-MM-dd'))"
}

# signs files in place with SHA-256 and a time stamp
function Invoke-Sign([string[]]$files) {
    & $signtool sign /sha1 $CertSha1 /tr $TimestampUrl /td sha256 /fd sha256 /v @files | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "signing failed: $($files -join ', ')" }
}

function Test-Signature([string]$file) {
    & $signtool verify /pa /all $file | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "signature not valid: $file" }
}

Write-Host "Building opensecagent $version"
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

python -I dev\third-party-licenses.py --check
if ($LASTEXITCODE -ne 0) { throw 'THIRD-PARTY-LICENSES.txt is out of date: python -I dev\third-party-licenses.py' }

$programs = @('target\release\opensecagent.exe', 'target\release\opensecagent-enroll.exe')
$isccArgs = @('/Qp', "/DAppVersion=$version")
if ($sign) {
    Invoke-Sign $programs
    # Inno Setup signs the installer, the uninstaller and its temporary copies with this command; it replaces $q by a
    # quote and $f by the file
    $isccArgs += '/DSign'
    $isccArgs += ('/Sopensecagent=$q{0}$q sign /sha1 {1} /tr {2} /td sha256 /fd sha256 $f' -f $signtool, $CertSha1, $TimestampUrl)
}

& $iscc @isccArgs 'packaging\windows\opensecagent.iss'
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }

$setup = Join-Path $root "target\installer\opensecagent-$version-x64-setup.exe"
$hash = (Get-FileHash -Algorithm SHA256 $setup).Hash.ToLower()
Write-Host ''
Write-Host "Installer: $setup"
if ($sign) {
    foreach ($file in $programs + $setup) { Test-Signature $file }
}
Write-Host "SHA-256:   $hash"
if ($sign) {
    Write-Host "Signed:    $($cert.Subject)"
} else {
    Write-Host 'Unsigned: SmartScreen warns, Smart App Control blocks it (docs/windows.md 6). Sign with -CertSha1 <thumbprint>.'
}
