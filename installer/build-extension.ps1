<#
.SYNOPSIS
    Assembles the RoomMate pyRevit extension, ready to install or to deploy.

.DESCRIPTION
    A checkout of this repository is not a loadable extension: lib\ is empty on
    purpose, because room_m has one source (extractor\pyRevit\room_m) and duHast
    has its own repository. This script is what turns the source into the thing
    pyRevit loads:

        fetch duHast at the commit in extractor\duhast.lock
        copy the bundles, room_m, and that duHast into one staged folder
        stamp both packages with what they are
        check the result the ways only a machine with python can

    Called by build.ps1 before the installer is compiled, and directly with
    -Deploy while developing.

    duHast is fetched, never committed here. Which duHast runs decides what an
    export MEANS -- an item's level, a door's footprint, whether a hole in a
    floor is a hole -- so it is pinned rather than picked up from whatever the
    machine happens to have, and every run prints the one it loaded.

.PARAMETER OutDir
    Where to assemble. Defaults to target\extension, and the folder is emptied
    first: a stale module left behind is a module that still imports.

.PARAMETER DuHastPath
    Use a local duHast (a path to src\duHast) instead of fetching the pinned
    commit. For developing the two together -- the stamp then records the local
    commit and marks it dirty when the working tree is.

.PARAMETER Deploy
    Also copy the result to %APPDATA%\pyRevit\Extensions\RoomMate.extension,
    which is where the installer puts it and where pyRevit looks. This is the
    development loop now that the buttons no longer live in the duHast repo.

.PARAMETER Version
    Version to stamp. Defaults to the crate version; build.ps1 passes the tag.
#>
[CmdletBinding()]
param(
    [string] $OutDir,
    [string] $DuHastPath,
    [switch] $Deploy,
    [string] $Version
)

$ErrorActionPreference = 'Stop'

$RepoRoot      = Split-Path -Parent $PSScriptRoot
$ExtensionSrc  = Join-Path $RepoRoot 'extractor\pyRevit\RoomMate.extension'
$RoomMSrc      = Join-Path $RepoRoot 'extractor\pyRevit\room_m'
$LockFile      = Join-Path $RepoRoot 'extractor\duhast.lock'
$CacheRoot     = Join-Path $RepoRoot 'target\duhast-cache'

if (-not $OutDir) { $OutDir = Join-Path $RepoRoot 'target\extension' }
$Staged = Join-Path $OutDir 'RoomMate.extension'

if (-not $Version) {
    $cargoToml = Get-Content (Join-Path $RepoRoot 'Cargo.toml') -Raw
    if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw "Could not read version from Cargo.toml" }
    $Version = $Matches[1]
}
$Version = $Version -replace '^v', ''

# ---------------------------------------------------------------- the stamp --

function Get-GitFacts($repoRoot, $scopePath) {
    # Returns @{ commit; dirty }. A failed probe is an expected outcome, not
    # this script's result -- without the reset a machine with no git ends the
    # build on git's exit code having built perfectly well.
    $facts = @{ commit = 'unknown'; dirty = $false }
    if (-not (Test-Path (Join-Path $repoRoot '.git'))) { return $facts }
    Push-Location $repoRoot
    try {
        $sha = git rev-parse HEAD 2>$null
        if ($LASTEXITCODE -eq 0 -and $sha) {
            $facts.commit = $sha.Trim()
            $changes = git status --porcelain -- $scopePath 2>$null
            $facts.dirty = [bool]$changes
        }
    } catch {
    } finally {
        Pop-Location
        $global:LASTEXITCODE = 0
    }
    return $facts
}

function Write-PythonStamp($path, $lines) {
    # ASCII, LF, no BOM: IronPython 2.7 refuses to parse a source file holding
    # any non-ASCII byte, and fails at import with a SyntaxError nowhere near
    # the character.
    $text = ($lines -join "`n") + "`n"
    $text = $text.Replace("`r`n", "`n")
    [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
}

$DescribeDuHast = @(
    ''
    ''
    'def describe():'
    '    """One line naming this copy, for a log or a tool output."""'
    '    if not COMMIT or COMMIT == "unknown":'
    '        return "unknown"'
    '    text = COMMIT[:8]'
    '    if DIRTY:'
    '        text += "-dirty"'
    '    if BRANCH and BRANCH != "unknown":'
    '        text += " (" + BRANCH + ")"'
    '    return text'
)

# --------------------------------------------------------------- the duHast --

function Get-DuHast {
    <#
        Returns @{ path; commit; dirty; branch; source }.

        The pinned commit is fetched into target\duhast-cache\<sha>, so the
        second build does no network at all. A local -DuHastPath wins, and says
        so in the stamp.
    #>
    if ($DuHastPath) {
        if (-not (Test-Path (Join-Path $DuHastPath '__init__.py'))) {
            throw "-DuHastPath '$DuHastPath' does not look like duHast (no __init__.py)."
        }
        $repo = Split-Path -Parent (Split-Path -Parent $DuHastPath)
        $facts = Get-GitFacts $repo 'src/duHast'
        $branch = 'unknown'
        if (Test-Path (Join-Path $repo '.git')) {
            Push-Location $repo
            try { $branch = (git rev-parse --abbrev-ref HEAD 2>$null); if ($LASTEXITCODE -ne 0) { $branch = 'unknown' } }
            finally { Pop-Location; $global:LASTEXITCODE = 0 }
        }
        return @{
            path = $DuHastPath; commit = $facts.commit; dirty = $facts.dirty
            branch = ($branch | ForEach-Object { "$_".Trim() }); source = "local: $DuHastPath"
        }
    }

    if (-not (Test-Path $LockFile)) { throw "Missing $LockFile" }
    $lock = Get-Content $LockFile -Raw | ConvertFrom-Json
    foreach ($field in 'repo', 'commit', 'path') {
        if (-not $lock.$field) { throw "duhast.lock has no '$field'" }
    }

    $cache = Join-Path $CacheRoot $lock.commit
    $marker = Join-Path $cache '.fetched'
    if (-not (Test-Path $marker)) {
        Write-Host "Fetching duHast $($lock.commit.Substring(0,8)) from $($lock.repo) ..." -ForegroundColor Cyan
        if (Test-Path $cache) { Remove-Item -Recurse -Force $cache }
        New-Item -ItemType Directory -Force -Path $cache | Out-Null
        # A bare init + fetch of the one commit: cloning that repository for a
        # subfolder pulls its DLLs and sample data as well.
        git -C $cache init --quiet 2>&1 | Out-Null
        git -C $cache remote add origin $lock.repo 2>&1 | Out-Null
        git -C $cache config core.sparseCheckout true
        git -C $cache sparse-checkout init --cone 2>&1 | Out-Null
        git -C $cache sparse-checkout set $lock.path 2>&1 | Out-Null
        git -C $cache fetch --depth 1 origin $lock.commit 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Could not fetch $($lock.commit) from $($lock.repo)" }
        git -C $cache checkout --quiet FETCH_HEAD 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Could not check out $($lock.commit)" }

        # The pin is only a pin if the thing fetched IS the commit named.
        $head = (git -C $cache rev-parse HEAD).Trim()
        if ($head -ne $lock.commit) { throw "Fetched $head but duhast.lock names $($lock.commit)" }
        New-Item -ItemType File -Path $marker -Force | Out-Null
    } else {
        Write-Host "Using cached duHast $($lock.commit.Substring(0,8))" -ForegroundColor DarkGray
    }

    $path = Join-Path $cache ($lock.path -replace '/', '\')
    if (-not (Test-Path (Join-Path $path '__init__.py'))) {
        throw "duhast.lock path '$($lock.path)' is not a python package in the fetched tree."
    }
    return @{
        path = $path; commit = $lock.commit; dirty = $false
        branch = $(if ($lock.branch) { $lock.branch } else { 'unknown' })
        source = "$($lock.repo) @ $($lock.commit.Substring(0,8))"
    }
}

# -------------------------------------------------------------- assemble it --

Write-Host "RoomMate extension $Version" -ForegroundColor Cyan

$duHast = Get-DuHast
Write-Host "duHast: $($duHast.source)" -ForegroundColor DarkGray

if (Test-Path $Staged) { Remove-Item -Recurse -Force $Staged }
New-Item -ItemType Directory -Force -Path $Staged | Out-Null

# The bundles. lib\README.md explains an empty folder and would be confusing
# next to the packages it describes, so it does not travel.
Copy-Item -Path (Join-Path $ExtensionSrc '*') -Destination $Staged -Recurse -Force `
    -Exclude '__pycache__'
$stagedLib = Join-Path $Staged 'lib'
Remove-Item -Force (Join-Path $stagedLib 'README.md') -ErrorAction SilentlyContinue

# room_m, from its one source.
Copy-Item -Path $RoomMSrc -Destination $stagedLib -Recurse -Force
Get-ChildItem -Path $stagedLib -Directory -Recurse -Filter '__pycache__' |
    Remove-Item -Recurse -Force

# duHast, without its binaries: nothing room_m reaches loads one, and
# check_duhast_closure.py is what keeps that true.
$stagedDuHast = Join-Path $stagedLib 'duHast'
Copy-Item -Path $duHast.path -Destination $stagedDuHast -Recurse -Force
Get-ChildItem -Path $stagedDuHast -Directory -Recurse -Filter '__pycache__' |
    Remove-Item -Recurse -Force
$binaries = Join-Path $stagedDuHast 'lib'
if (Test-Path $binaries) { Remove-Item -Recurse -Force $binaries }

# The two stamps. duHast's matches what updateDuHastInPyRevitSample.ps1 writes
# in the duHast repo, field for field, so room_m reads one shape either way.
$roomMate = Get-GitFacts $RepoRoot 'extractor'
Write-PythonStamp (Join-Path $stagedDuHast '_build_info.py') (@(
    '"""Records which duHast this copy is, and what deployed it.'
    ''
    'Written into the copy, never into the source tree: a duHast with no'
    '_build_info.py is one nobody deployed, which reads as "unknown" rather'
    'than as an error. Read it through describe() so every caller words it the'
    'same.'
    '"""'
    ''
    ('COMMIT = "{0}"' -f $duHast.commit)
    ('DIRTY = {0}' -f $(if ($duHast.dirty) { 'True' } else { 'False' }))
    ('BRANCH = "{0}"' -f $duHast.branch)
    ('BUILT_AT = "{0}"' -f (Get-Date).ToString('yyyy-MM-ddTHH:mm:sszzz'))
    'BUILT_BY = "RoomMate build-extension.ps1"'
) + $DescribeDuHast)

Write-PythonStamp (Join-Path $stagedLib 'room_m\_build_info.py') @(
    '"""Records which RoomMate extractor this is, and which duHast it was built'
    'against.'
    ''
    'DUHAST_COMMIT is the pin from extractor/duhast.lock at build time.'
    'room_m.utils.provenance compares it against the duHast that actually'
    'loaded, which is not the same question: two extensions can each carry a'
    'package called duHast, and the one already imported in a Revit session'
    'wins whatever the path says.'
    '"""'
    ''
    ('VERSION = "{0}"' -f $Version)
    ('COMMIT = "{0}"' -f $roomMate.commit)
    ('DIRTY = {0}' -f $(if ($roomMate.dirty) { 'True' } else { 'False' }))
    ('DUHAST_COMMIT = "{0}"' -f $duHast.commit)
    ('BUILT_AT = "{0}"' -f (Get-Date).ToString('yyyy-MM-ddTHH:mm:sszzz'))
    'BUILT_BY = "build-extension.ps1"'
    ''
    ''
    'def describe():'
    '    """One line naming this build, for a log or a tool output."""'
    '    text = VERSION'
    '    if COMMIT and COMMIT != "unknown":'
    '        text += " (" + COMMIT[:8]'
    '        if DIRTY:'
    '            text += "-dirty"'
    '        text += ")"'
    '    elif DIRTY:'
    '        text += " (dirty)"'
    '    return text'
)

# extension.json carries a version so pyRevit and anyone reading the installed
# folder see the same one the installer does.
$manifestPath = Join-Path $Staged 'extension.json'
$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
$manifest.version = $Version
Write-PythonStamp $manifestPath @(($manifest | ConvertTo-Json -Depth 10))

# ----------------------------------------------------------------- check it --

$python = Get-Command 'python' -ErrorAction SilentlyContinue
if (-not $python) { $python = Get-Command 'py' -ErrorAction SilentlyContinue }
if (-not $python) {
    # Packaging deliberately needs no node; python is different -- these checks
    # are the only thing standing between a bad bundle and a Revit session.
    throw "python not found, and the extension checks need it."
}

& $python.Source (Join-Path $RepoRoot 'scripts\check_extension.py')
if ($LASTEXITCODE -ne 0) { throw 'extension source checks failed' }

& $python.Source (Join-Path $RepoRoot 'scripts\check_duhast_closure.py') $stagedLib
if ($LASTEXITCODE -ne 0) { throw 'duHast closure checks failed' }

# room_m as STAGED, rather than trusting the source check above to cover what
# was copied. duHast's own files are the closure check's business: it reports
# the reachable ones with no encoding cookie and leaves the call upstream.
$nonAscii = @()
Get-ChildItem -Path (Join-Path $stagedLib 'room_m') -Recurse -Filter '*.py' | ForEach-Object {
    $bytes = [System.IO.File]::ReadAllBytes($_.FullName)
    if ($bytes | Where-Object { $_ -gt 127 }) { $nonAscii += $_.FullName }
}
if ($nonAscii) { throw "Non-ASCII bytes in extractor python:`n" + ($nonAscii -join "`n") }

$fileCount = (Get-ChildItem -Path $Staged -Recurse -File).Count
Write-Host "Staged $fileCount files in $Staged" -ForegroundColor Green

# ---------------------------------------------------------------- deploy it --

if ($Deploy) {
    $target = Join-Path $env:APPDATA 'pyRevit\Extensions\RoomMate.extension'
    if (Test-Path $target) { Remove-Item -Recurse -Force $target }
    New-Item -ItemType Directory -Force -Path $target | Out-Null
    Copy-Item -Path (Join-Path $Staged '*') -Destination $target -Recurse -Force
    Write-Host "Deployed to $target" -ForegroundColor Green
    Write-Host "Restart Revit (or reload pyRevit) to pick it up." -ForegroundColor Yellow
}
