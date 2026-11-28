# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# win-msi.ps1 -Version <X.Y.Z[-dev]> -Bin <dir> -Out <grind.msi>
#
# The Windows installer, packaging/windows/grind.wxs, around the grind-win32.exe, grind.exe and
# grind-tui.exe in <dir>; ICE-validated, then installed on this runner the way a user would and
# checked at every step:
#
#   * an older package installs, and this one upgrades it: one Grind in Installed apps, at this
#     version, its programs on the user's PATH once and "Open with" pointing at them;
#   * the same version again replaces itself, and the older one is refused afterwards;
#   * uninstalling leaves no file, no PATH entry, no shortcut and no registry key behind.
#
# Needs `wix` (5.x) on PATH. Runs the installer per user, so needs no administrator.

param(
    [Parameter(Mandatory)] [string] $Version,
    [Parameter(Mandatory)] [string] $Bin,
    [Parameter(Mandatory)] [string] $Out
)

$ErrorActionPreference = "Stop"
$upgradeCode = "{01071E58-5739-409E-8EF1-36B799BCF365}"
$number = ($Version -split '-')[0]
$wxs = "packaging/windows/grind.wxs"
$data = "ui_win32/data"

function Build([string] $number, [string] $out) {
    wix build -arch x64 -d "Version=$number" -d "Bin=$Bin" -d "Data=$data" $wxs -o $out
    if ($LASTEXITCODE -ne 0) { throw "wix build $number failed" }
}

function Msiexec([string] $verb, [string] $msi, [string] $log) {
    $p = Start-Process msiexec -ArgumentList "/$verb `"$msi`" /qn /norestart /l*v `"$log`"" -Wait -PassThru
    return $p.ExitCode
}

function Must([bool] $ok, [string] $message) {
    if (-not $ok) { throw $message }
    Write-Host "OK - $message"
}

$installer = New-Object -ComObject WindowsInstaller.Installer
function Installed {
    return @($installer.RelatedProducts($upgradeCode) | ForEach-Object {
        $installer.ProductInfo($_, "VersionString")
    })
}

$folder = Join-Path $env:LOCALAPPDATA "Programs\Grind"
$shortcut = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Grind.lnk"
function UserPath { return [Environment]::GetEnvironmentVariable("Path", "User") }
function OnPath { return @((UserPath) -split ';' | Where-Object { $_.TrimEnd('\') -eq $folder }).Count }

Build $number $Out
$older = Join-Path ([IO.Path]::GetTempPath()) "grind-older.msi"
Build "0.0.1" $older

wix msi validate $Out
if ($LASTEXITCODE -ne 0) { throw "ICE validation of $Out failed" }
Write-Host "OK - $Out passes ICE validation"

Must ((Installed).Count -eq 0) "no Grind installed before"

Must ((Msiexec i $older older.log) -eq 0) "the older package installs"
Must (((Installed) -join ',') -eq "0.0.1") "it is in Installed apps"

Must ((Msiexec i $Out upgrade.log) -eq 0) "this package installs over it"
Must (((Installed) -join ',') -eq $number) "one Grind installed, at $number"
foreach ($exe in "grind-win32.exe", "grind.exe", "grind-tui.exe") {
    Must (Test-Path (Join-Path $folder $exe)) "$exe is in $folder"
}
$said = & (Join-Path $folder "grind.exe") --version | Select-Object -First 1
Must ($said -eq "grind $Version (release build)") "the installed grind says it is $Version"
Must ((OnPath) -eq 1) "$folder is on the user's PATH once"
Must (Test-Path $shortcut) "the Start menu has Grind"
Must (Test-Path "HKCU:\Software\Grind\Capabilities") "Grind is offered in Default apps"
$command = (Get-ItemProperty "HKCU:\Software\Classes\Applications\grind-win32.exe\shell\open\command")."(default)"
Must ($command -like "*$folder\grind-win32.exe*") "Open with runs the installed copy: $command"

Must ((Msiexec i $Out again.log) -eq 0) "the same version installs again"
Must (((Installed) -join ',') -eq $number) "and is still one Grind"
Must ((OnPath) -eq 1) "still on PATH once"

Must ((Msiexec i $older downgrade.log) -ne 0) "the older package is refused now"
Must (((Installed) -join ',') -eq $number) "and changed nothing"

Must ((Msiexec x $Out uninstall.log) -eq 0) "it uninstalls"
Must ((Installed).Count -eq 0) "no Grind installed after"
Must (-not (Test-Path $folder)) "$folder is gone"
Must ((OnPath) -eq 0) "the PATH entry is gone"
Must (-not (Test-Path $shortcut)) "the shortcut is gone"
Must (-not (Test-Path "HKCU:\Software\Grind")) "HKCU\Software\Grind is gone"
Must (-not (Test-Path "HKCU:\Software\Classes\Applications\grind-win32.exe")) "Open with no longer offers Grind"
