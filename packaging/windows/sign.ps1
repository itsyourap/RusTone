# Signs files with the Authenticode certificate in WINDOWS_CERTIFICATE (a
# base64-encoded .pfx) and WINDOWS_CERTIFICATE_PASSWORD, timestamped.
# Needs signtool on PATH (the Visual Studio developer environment).
#
#   ./sign.ps1 target/release/rustone.exe, dist/packages/RusTone_x64-setup.exe
param([Parameter(Mandatory)][string[]] $Files)

$ErrorActionPreference = "Stop"
if (-not $env:WINDOWS_CERTIFICATE) { throw "WINDOWS_CERTIFICATE is not set" }

$pfx = Join-Path ([IO.Path]::GetTempPath()) "rustone-signing.pfx"
[IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE))
try {
    foreach ($file in $Files) {
        signtool sign /f $pfx /p $env:WINDOWS_CERTIFICATE_PASSWORD /fd sha256 /tr http://timestamp.digicert.com /td sha256 /d RusTone $file
        if ($LASTEXITCODE) { throw "signtool failed for $file" }
    }
} finally {
    Remove-Item $pfx -Force
}
