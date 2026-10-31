<#
.SYNOPSIS
    Registers RoomMate's MCP servers with Claude Code, so they work with no setup by hand.

.DESCRIPTION
    Claude Code keeps its MCP servers in its own configuration, and the only supported way to add
    one is `claude mcp add`. This runs that for the installed copy, with the real paths filled in
    (the command takes none of %LOCALAPPDATA% and the like, and PowerShell does not expand them
    inside it), at user scope so the servers are there in every folder.

    * No switch: registers the RoomMate data server (mcp.exe) against this install's settings.
    * -Url: registers a RoomMate server somebody else runs on the network instead (see
      docs/mcp-host-setup.md, 2b), so a colleague's whole setup is one command.
    * -Drofus: asks for the dRofus database, project and API key, then registers drofus-mcp.exe.
      Nothing can fill those in beforehand, which is why this one is a question and not a default.
    * -Remove: takes both back out. Run by the uninstaller.

    Registering is repeatable: an existing entry of the same name is replaced, so running it again
    after an upgrade or a move is the way to repair it.

    A missing Claude Code is not an error. This is run by the installer, which must not fail
    because a tool is absent; it says what to do and stops.

.PARAMETER Url
    The address of a shared RoomMate MCP server, e.g. http://192.168.1.20:5152/mcp.

.PARAMETER Token
    The shared secret that server was started with. Sent as a bearer token. It is stored in
    Claude Code's configuration file in plain text, like every header it is given.

.PARAMETER Drofus
    Register the dRofus server instead (prompts).

.PARAMETER Remove
    Remove the registrations this script makes.

.PARAMETER DryRun
    Print what would be run and change nothing. The API key is shown as ***.
#>
[CmdletBinding()]
param(
    [string] $Url,
    [string] $Token,
    [switch] $Drofus,
    [switch] $Remove,
    [switch] $DryRun
)

$ErrorActionPreference = 'Stop'

$AppRoot  = $PSScriptRoot
$Settings = Join-Path $env:LOCALAPPDATA 'RoomMate\settings'

function Find-Claude {
    $found = Get-Command claude -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    # Where the native installer and npm put it when the shell's PATH has not caught up yet, which
    # is the usual state straight after installing Claude Code.
    foreach ($candidate in @(
            (Join-Path $env:USERPROFILE '.local\bin\claude.exe'),
            (Join-Path $env:APPDATA 'npm\claude.cmd'))) {
        if (Test-Path $candidate) { return $candidate }
    }
    return $null
}

function Stop-Here {
    param([string] $Message, [int] $Code = 0)
    Write-Host $Message
    # Hold the window only when somebody can read it: the installer runs this in its own console.
    if ([Environment]::UserInteractive) { Read-Host 'Press Enter to close' | Out-Null }
    exit $Code
}

$Claude = Find-Claude

if (-not $Claude) {
    if ($Remove) { exit 0 }
    Stop-Here @"
Claude Code was not found, so nothing was registered.
Install it, then run this again:
  powershell -ExecutionPolicy Bypass -File "$PSCommandPath"
"@
}

# Runs `claude` with an argument list. $Shown replaces secrets in what is printed.
function Invoke-Claude {
    param([string[]] $Arguments, [string[]] $Shown)
    if (-not $Shown) { $Shown = $Arguments }
    if ($DryRun) {
        Write-Host ("claude " + ($Shown -join ' '))
        return $true
    }
    # Windows PowerShell 5.1 turns a native command's stderr into a terminating error under
    # 'Stop', and removing a server that is not there writes to stderr.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & $Claude @Arguments *> $null } catch { return $false } finally { $ErrorActionPreference = $previous }
    return ($LASTEXITCODE -eq 0)
}

function Add-Server {
    param([string] $Name, [string[]] $AddArguments, [string[]] $Shown)
    # Replace, so a second run repairs rather than failing on "already exists".
    Invoke-Claude -Arguments @('mcp', 'remove', $Name, '--scope', 'user') | Out-Null
    if (Invoke-Claude -Arguments $AddArguments -Shown $Shown) {
        Write-Host "Registered '$Name' with Claude Code."
    } else {
        Stop-Here "Could not register '$Name'. Run 'claude mcp add --help' to see what Claude Code expects." 1
    }
}

if ($Remove) {
    foreach ($name in 'roommate', 'drofus') {
        Invoke-Claude -Arguments @('mcp', 'remove', $name, '--scope', 'user') | Out-Null
    }
    exit 0
}

if ($Drofus) {
    $Exe = Join-Path $AppRoot 'drofus-mcp.exe'
    if (-not (Test-Path $Exe)) { Stop-Here "drofus-mcp.exe was not found beside this script ($AppRoot)." 1 }

    Write-Host 'dRofus: these three values come from your dRofus account.'
    Write-Host 'Region host: api-no (default), api-eu, api-us, api-ca or api-au.'
    $region = Read-Host 'Region [api-no]'
    if (-not $region) { $region = 'api-no' }
    $database = Read-Host 'Database'
    $project  = Read-Host 'Project'
    $secure   = Read-Host 'API key (not shown)' -AsSecureString
    $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try { $key = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr) }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
    if (-not $database -or -not $project -or -not $key) {
        Stop-Here 'Database, project and API key are all needed; nothing was registered.' 1
    }

    # The name goes first: --env takes any number of values and would swallow it otherwise.
    $tail = @('--', $Exe, '--base-url', "https://$region.drofus.com", '--database', $database, '--project', $project)
    $head = @('mcp', 'add', 'drofus', '--scope', 'user', '--env')
    Add-Server -Name 'drofus' `
        -AddArguments ($head + @("DROFUS_API_KEY=$key") + $tail) `
        -Shown        ($head + @('DROFUS_API_KEY=***') + $tail)
    Write-Host 'The API key is stored in Claude Code''s configuration file.'
    Stop-Here 'Done. Restart Claude Code to pick it up.'
}

if ($Url) {
    $head = @('mcp', 'add', '--transport', 'http', '--scope', 'user', 'roommate', $Url)
    if ($Token) {
        Add-Server -Name 'roommate' `
            -AddArguments ($head + @('--header', "Authorization: Bearer $Token")) `
            -Shown        ($head + @('--header', 'Authorization: Bearer ***'))
    } else {
        Add-Server -Name 'roommate' -AddArguments $head
    }
    exit 0
}

$Exe = Join-Path $AppRoot 'mcp.exe'
if (-not (Test-Path $Exe)) { Stop-Here "mcp.exe was not found beside this script ($AppRoot)." 1 }
Add-Server -Name 'roommate' -AddArguments @(
    'mcp', 'add', 'roommate', '--scope', 'user', '--',
    $Exe,
    '--server-settings',  (Join-Path $Settings 'server.toml'),
    '--project-settings', (Join-Path $Settings 'projects'))
exit 0
