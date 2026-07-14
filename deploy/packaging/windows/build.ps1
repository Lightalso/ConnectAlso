# ConnectAlso Windows Build & Packaging
# =========================================
#
# Prerequisites:
#   - Rust 1.85+ (https://rustup.rs)
#   - WiX Toolset v4 (https://wixtoolset.org)
#   - Windows SDK (signtool.exe)
#   - Wintun DLL (https://wintun.net)
#
# Usage: powershell -File deploy\packaging\windows\build.ps1 -Version "0.1.0"

param(
    [string]$Version = "0.1.0",
    [string]$OutputDir = "target\release",
    [string]$SignCert = "",
    [string]$SignPassword = ""
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))

Write-Host "ConnectAlso Windows Build v$Version" -ForegroundColor Cyan

# 1. Build Rust binaries
Write-Host "[1/5] Building Rust binaries..." -ForegroundColor Yellow
Push-Location $Root
cargo build --release -p connectalso-control -p connectalso-relay -p connectalso-stun -p connectalso-daemon -p connectalso-cli -p connectalso-desktop
if ($LASTEXITCODE -ne 0) { throw "Build failed" }
Pop-Location

# 2. Gather artifacts
Write-Host "[2/5] Gathering artifacts..." -ForegroundColor Yellow
$StageDir = "$OutputDir\connectalso-$Version"
Remove-Item -Recurse -Force $StageDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $StageDir\bin -Force | Out-Null
New-Item -ItemType Directory -Path $StageDir\driver -Force | Out-Null
New-Item -ItemType Directory -Path $StageDir\config -Force | Out-Null

Copy-Item "$OutputDir\connectalso-control.exe"  "$StageDir\bin\"
Copy-Item "$OutputDir\connectalso-relay.exe"    "$StageDir\bin\"
Copy-Item "$OutputDir\connectalso-stun.exe"     "$StageDir\bin\"
Copy-Item "$OutputDir\connectalso-daemon.exe"   "$StageDir\bin\"
Copy-Item "$OutputDir\connectalso.exe"          "$StageDir\bin\"
Copy-Item "$OutputDir\connectalso-desktop.exe"  "$StageDir\bin\"

# Copy README and LICENSE
Copy-Item "$Root\README.md" $StageDir\
Copy-Item "$Root\LICENSE"   $StageDir\

# Wintun driver note
@"
Wintun Driver
=============
Download wintun.dll from https://www.wintun.net/
Place in C:\Windows\System32\ or next to connectalso-daemon.exe
"@ | Out-File -FilePath "$StageDir\driver\README.txt" -Encoding UTF8

# 3. Sign binaries (if certificate provided)
if ($SignCert -and $SignPassword) {
    Write-Host "[3/5] Signing binaries..." -ForegroundColor Yellow
    Get-ChildItem "$StageDir\bin\*.exe" | ForEach-Object {
        & signtool sign /f $SignCert /p $SignPassword /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $_.FullName
    }
} else {
    Write-Host "[3/5] Skipping code signing (no certificate)" -ForegroundColor DarkYellow
}

# 4. Create MSI installer with WiX
Write-Host "[4/5] Creating MSI installer..." -ForegroundColor Yellow

$WixSource = @"
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="ConnectAlso" Manufacturer="ConnectAlso Contributors"
           Version="$Version" UpgradeCode="A1B2C3D4-E5F6-7890-ABCD-EF1234567890"
           Scope="perMachine">
    <MajorUpgrade DowngradeErrorMessage="A newer version is already installed." />
    <MediaTemplate EmbedCab="yes" />

    <Feature Id="Main" Title="ConnectAlso" Level="1">
      <ComponentGroupRef Id="Binaries" />
    </Feature>

    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="ProgramFiles64Folder">
        <Directory Id="INSTALLDIR" Name="ConnectAlso">
          <ComponentGroup Id="Binaries" Directory="INSTALLDIR">
            $(Get-ChildItem "$StageDir\bin\*.exe" | ForEach-Object {
              "<Component><File Source=`"$($_.FullName)`" /></Component>"
            })
          </ComponentGroup>
        </Directory>
      </Directory>
      <Directory Id="ProgramMenuFolder">
        <Directory Id="StartMenuDir" Name="ConnectAlso" />
      </Directory>
    </Directory>

    <!-- Register service -->
    <ServiceInstall Id="DaemonService" Type="ownProcess" Name="ConnectAlsoDaemon"
                    DisplayName="ConnectAlso Daemon" Description="ConnectAlso Virtual Network Daemon"
                    Start="auto" ErrorControl="normal"
                    Arguments='--control-url http://127.0.0.1:3000 --stun-server 127.0.0.1:3478 --relay-server 127.0.0.1:33478' />
    <ServiceControl Id="DaemonControl" Name="ConnectAlsoDaemon" Start="install" Stop="both" Remove="uninstall" />
  </Package>
</Wix>
"@

$WixSource | Out-File -FilePath "$StageDir\connectalso.wxs" -Encoding UTF8
Push-Location $StageDir
wix build connectalso.wxs -o "connectalso-$Version-x64.msi"
Pop-Location

# 5. Create portable ZIP
Write-Host "[5/5] Creating portable ZIP..." -ForegroundColor Yellow
Compress-Archive -Path "$StageDir\*" -DestinationPath "$OutputDir\connectalso-$Version-portable.zip" -Force

Write-Host ""
Write-Host "Build complete!" -ForegroundColor Green
Write-Host "  MSI     : $OutputDir\connectalso-$Version-x64.msi"
Write-Host "  ZIP     : $OutputDir\connectalso-$Version-portable.zip"
