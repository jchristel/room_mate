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
      The key goes into the Windows Credential Manager (drofus-mcp.exe stores it itself, reading it from
      standard input); if that is unavailable, into a file only your account can read (under
      %LOCALAPPDATA%\RoomMate\secrets). Either way Claude Code's own configuration never holds the key.
    * -Remove: takes both back out. Run by the uninstaller.

    Registering is repeatable: an existing entry of the same name is replaced, so running it again
    after an upgrade or a move is the way to repair it.

    A missing Claude Code is not an error. This is run by the installer, which must not fail
    because a tool is absent; it says what to do and stops.

.PARAMETER Url
    The address of a shared RoomMate MCP server, e.g. http://192.168.1.20:5152/mcp.

.PARAMETER Token
    The shared secret that server was started with. Sent as a bearer token. Deprecated as a
    parameter: a command line shows in history and the process list. Use -AskToken, or set
    ROOMMATE_MCP_TOKEN. Claude Code stores the header in its configuration file in plain text,
    like every header it is given.

.PARAMETER AskToken
    Prompt for the shared secret (nothing is shown) instead of taking it on the command line.

.PARAMETER Drofus
    Register the dRofus server instead (prompts).

.PARAMETER Unattended
    With -Drofus: take region, database, project and API key from the ROOMMATE_SETUP_REGION, _DATABASE, _PROJECT
    and _KEY environment variables instead of asking, and never wait for a key press. The installer's own page
    collects them and sets them for this one process, so the key is on no command line.

.PARAMETER Remove
    Remove the registrations this script makes.

.PARAMETER DryRun
    Print what would be run and change nothing. The API key is shown as ***.
#>
[CmdletBinding()]
param(
    [string] $Url,
    [string] $Token,
    [switch] $AskToken,
    [switch] $Drofus,
    [switch] $Unattended,
    [switch] $Remove,
    [switch] $DryRun
)

$ErrorActionPreference = 'Stop'

$AppRoot  = $PSScriptRoot
$Settings = Join-Path $env:LOCALAPPDATA 'RoomMate\settings'
# Where the dRofus key is kept: a file only this account can read, so Claude Code's configuration holds a path.
$KeyFile  = Join-Path $env:LOCALAPPDATA 'RoomMate\secrets\drofus.key'

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
    if ([Environment]::UserInteractive -and -not $Unattended) { Read-Host 'Press Enter to close' | Out-Null }
    exit $Code
}

$Claude = Find-Claude

if (-not $Claude) {
    if ($Remove) { exit 0 }
    # Exit code 2 is "no Claude Code" to the installer, which is running this hidden and says so in a message box.
    $missingCode = 0
    if ($Unattended) { $missingCode = 2 }
    Stop-Here -Code $missingCode -Message @"
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
    if (-not $DryRun) {
        Remove-Item -LiteralPath $KeyFile -Force -ErrorAction SilentlyContinue
        $drofusExe = Join-Path $AppRoot 'drofus-mcp.exe'
        if (Test-Path $drofusExe) {
            $previous = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
            try { & $drofusExe --clear-key *> $null } catch { } finally { $ErrorActionPreference = $previous }
        }
    }
    exit 0
}

# Keeps the dRofus key where Claude Code's own (plain text) configuration need not hold it, and returns the `--env`
# values the registration needs. First choice: the OS credential store, written by drofus-mcp.exe itself with the key
# on standard input, so it is on no command line. Fallback: a file only this account can read.
function Save-DrofusKey {
    param([string] $Key, [string] $Exe)
    if ($DryRun) {
        Write-Host '(would store the API key in the Windows Credential Manager, else in a file only you can read)'
        return @{ Where = 'the Windows Credential Manager'; Env = @() }
    }
    $global:LASTEXITCODE = 1
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $Key | & $Exe --store-key *> $null } catch { } finally { $ErrorActionPreference = $previous }
    if ($LASTEXITCODE -eq 0) { return @{ Where = 'the Windows Credential Manager'; Env = @() } }

    New-Item -ItemType Directory -Force -Path (Split-Path $KeyFile) | Out-Null
    # Empty first and locked down before the key is written, so it is never readable by anyone else.
    Set-Content -LiteralPath $KeyFile -Value '' -NoNewline
    & icacls.exe $KeyFile /inheritance:r /grant:r "${env:USERNAME}:(R,W,D)" *> $null
    if ($LASTEXITCODE -ne 0) {
        Remove-Item -LiteralPath $KeyFile -Force
        Stop-Here 'Could not restrict the key file to your account; nothing was stored.' 1
    }
    Set-Content -LiteralPath $KeyFile -Value $Key -NoNewline
    return @{ Where = "$KeyFile (readable by your account only)"; Env = @("DROFUS_API_KEY_FILE=$KeyFile") }
}

if ($Drofus) {
    $Exe = Join-Path $AppRoot 'drofus-mcp.exe'
    if (-not (Test-Path $Exe)) { Stop-Here "drofus-mcp.exe was not found beside this script ($AppRoot)." 1 }

    if ($Unattended) {
        $region = $env:ROOMMATE_SETUP_REGION
        $database = $env:ROOMMATE_SETUP_DATABASE
        $project = $env:ROOMMATE_SETUP_PROJECT
        $key = $env:ROOMMATE_SETUP_KEY
        # The values were for this process alone; do not leave the key in its environment.
        foreach ($name in 'REGION', 'DATABASE', 'PROJECT', 'KEY') { Remove-Item "Env:ROOMMATE_SETUP_$name" -ErrorAction SilentlyContinue }
    } else {
        Write-Host 'dRofus: these three values come from your dRofus account.'
        Write-Host 'Region host: api-no (default), api-eu, api-us, api-ca or api-au.'
        $region = Read-Host 'Region [api-no]'
        $database = Read-Host 'Database'
        $project  = Read-Host 'Project'
        $secure   = Read-Host 'API key (not shown)' -AsSecureString
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
        try { $key = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr) }
        finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
    }
    if (-not $region) { $region = 'api-no' }
    if ($region -notmatch '^[A-Za-z0-9-]+$') { Stop-Here "The region '$region' is not a host name part such as api-no; nothing was registered." 1 }
    if (-not $database -or -not $project -or -not $key) {
        Stop-Here 'Database, project and API key are all needed; nothing was registered.' 1
    }

    $saved = Save-DrofusKey -Key $key -Exe $Exe

    # The name goes first: --env takes any number of values and would swallow it otherwise.
    $tail = @('--', $Exe, '--base-url', "https://$region.drofus.com", '--database', $database, '--project', $project)
    $head = @('mcp', 'add', 'drofus', '--scope', 'user')
    if ($saved.Env.Count -gt 0) { $head = $head + @('--env') + $saved.Env }
    Add-Server -Name 'drofus' -AddArguments ($head + $tail) -Shown ($head + $tail)
    Write-Host "The API key is stored in $($saved.Where). Claude Code's configuration does not hold it."
    Stop-Here 'Done. Restart Claude Code to pick it up.'
}

if ($Url) {
    if (-not $Token) { $Token = $env:ROOMMATE_MCP_TOKEN }
    if ($Token -and $PSBoundParameters.ContainsKey('Token')) {
        Write-Warning '-Token puts the secret in your shell history; use -AskToken or ROOMMATE_MCP_TOKEN.'
    }
    if ($AskToken -and -not $Token) {
        $secureToken = Read-Host 'Shared secret (not shown)' -AsSecureString
        $tb = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureToken)
        try { $Token = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($tb) }
        finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($tb) }
    }
    $head = @('mcp', 'add', '--transport', 'http', '--scope', 'user', 'roommate', $Url)
    if ($Token) {
        Add-Server -Name 'roommate' `
            -AddArguments ($head + @('--header', "Authorization: Bearer $Token")) `
            -Shown        ($head + @('--header', 'Authorization: Bearer ***'))
        Write-Host 'Claude Code stores this header in its configuration file in plain text.'
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
