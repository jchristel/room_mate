<#
.SYNOPSIS
    Starts the RoomMate server and opens the viewer in the default browser.

.DESCRIPTION
    This is the shortcut's target -- the one thing an installed RoomMate is
    launched by. It exists because the server binary alone is not a usable
    Windows app: it needs its settings paths passed on the command line, its
    writable data root has to exist, and somebody has to open the browser once
    the port is actually accepting connections.

    Three properties are deliberate, and each replaces a way this goes wrong:

    * The server runs in the FOREGROUND of this console, so closing the window
      stops it. A background start would leave an invisible process holding
      port 5151 with no obvious way to stop it.
    * The browser is opened by a hidden watcher that WAITS for the port. Opening
      it immediately races the server's startup and lands on a connection error
      often enough to look broken.
    * Settings are re-seeded from the install directory's template when missing,
      so deleting the data folder is a reset rather than a broken install.

.PARAMETER Port
    TCP port to serve on. 5151 is the default everything else assumes -- the MCP
    binary's --server-url and any configured pyRevit pusher both point at it, so
    moving it here means moving it there too.

.PARAMETER NoBrowser
    Start the server without opening a browser.
#>
[CmdletBinding()]
param(
    [int] $Port = 5151,
    [switch] $NoBrowser
)

$ErrorActionPreference = 'Stop'

$AppRoot   = $PSScriptRoot
$DataRoot  = Join-Path $env:LOCALAPPDATA 'RoomMate'
$Settings  = Join-Path $DataRoot 'settings'
$Template  = Join-Path $AppRoot 'settings-template'
$Exe       = Join-Path $AppRoot 'roommate.exe'
$Url       = "http://127.0.0.1:$Port"

try { $Host.UI.RawUI.WindowTitle = "RoomMate server ($Port)" } catch { }

# A plain TcpClient connect rather than Test-NetConnection: this is called in a
# 250 ms poll loop below and Test-NetConnection takes seconds per probe.
function Test-Listening {
    param([int] $ProbePort)
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $client.Connect('127.0.0.1', $ProbePort)
        return $true
    } catch {
        return $false
    } finally {
        $client.Dispose()
    }
}

if (-not (Test-Path $Exe)) {
    Write-Host "roommate.exe is missing from $AppRoot -- reinstall RoomMate." -ForegroundColor Red
    exit 1
}

# Seed-if-absent, never overwrite: the installer does this too, but doing it
# here as well is what makes a deleted data folder self-heal, and it is the only
# thing standing between an edited project file and an upgrade that clobbers it.
if (-not (Test-Path (Join-Path $Settings 'server.toml'))) {
    Write-Host "Setting up $DataRoot ..."
    New-Item -ItemType Directory -Force -Path $Settings | Out-Null
    if (Test-Path $Template) {
        Copy-Item -Path (Join-Path $Template '*') -Destination $Settings -Recurse -Force
    }
}
New-Item -ItemType Directory -Force -Path (Join-Path $DataRoot 'data\snapshots') | Out-Null

# Double-clicking the shortcut twice should show the viewer, not a port clash.
if (Test-Listening $Port) {
    Write-Host "RoomMate is already running on $Url -- opening the viewer."
    if (-not $NoBrowser) { Start-Process $Url }
    exit 0
}

if (-not $NoBrowser) {
    # Base64 rather than a quoted -Command string: the watcher is multi-line and
    # carries both quote flavours, and -EncodedCommand is the one form that
    # survives that without escaping games. The 60 s deadline keeps a failed
    # start from leaving a poll loop running forever.
    $watcher = @"
`$deadline = (Get-Date).AddSeconds(60)
while ((Get-Date) -lt `$deadline) {
    `$c = New-Object System.Net.Sockets.TcpClient
    try {
        `$c.Connect('127.0.0.1', $Port)
        `$c.Dispose()
        Start-Process '$Url'
        exit
    } catch {
        `$c.Dispose()
    }
    Start-Sleep -Milliseconds 250
}
"@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($watcher))
    Start-Process -FilePath 'powershell.exe' -WindowStyle Hidden `
        -ArgumentList '-NoProfile', '-EncodedCommand', $encoded | Out-Null
}

Write-Host "RoomMate  ->  $Url"
Write-Host "Close this window to stop the server."
Write-Host ''

& $Exe --server-settings (Join-Path $Settings 'server.toml') `
       --project-settings (Join-Path $Settings 'projects') `
       --port $Port

# A shortcut-launched window vanishing on a startup error takes the error
# message with it, which is the difference between "port is taken" and "RoomMate
# does not work". Hold the window open only on failure.
if ($LASTEXITCODE -ne 0) {
    Write-Host ''
    Write-Host "RoomMate exited with code $LASTEXITCODE." -ForegroundColor Red
    Write-Host 'Press any key to close...'
    try { $null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown') } catch { Start-Sleep -Seconds 20 }
}
