<#
.SYNOPSIS
    Builds RoomMate in place -- binaries AND viewer -- and optionally runs it.

.DESCRIPTION
    `cargo build` is not the whole product, and the gap is silent rather than
    loud. The viewer's renderer bundle and the React settings page are generated
    by `npm run build` and deliberately COMMITTED, so a fresh clone plus
    `cargo run` serves a working viewer with no node installed. Cargo knows
    nothing about either, so the two drift apart with no error anyone can act on
    -- a frontend feature that is simply absent while the binary, the data and
    the source are all correct. This script is the one command that keeps them
    in step.

    IN PLACE, not packaged: the output is target\<profile>\, runnable from this
    repo. `installer\build.ps1` is the other build script -- it compiles the
    same two binaries and wraps them in a Windows installer for someone else's
    machine. Reach for that one when you are handing the product over, this one
    when you are running it yourself.

.PARAMETER Dev
    Build the debug profile instead of release. Faster to compile, and a debug
    build has no static\ beside it -- so it falls back to the repo's own and
    picks up an edited viewer file on a browser reload with no rebuild at all.
    That is the loop the viewer is actually developed in.

.PARAMETER Check
    Also run the CI gates -- the same set rust.yml and frontend.yml run, so a
    green run here is a green PR. Off by default because this script is for
    building; -Check is for before you push.

.PARAMETER SkipFrontend
    Skip npm entirely and build against whatever bundle is already committed.
    Correct when you have not touched src-js\ (or have no node installed);
    wrong, and quietly so, when you have.

.PARAMETER Run
    Start the server when the build succeeds, with the two settings flags it
    cannot start without.

.PARAMETER Port
    Port for -Run. Defaults to 5151, which is what .claude\launch.json pins and
    what bin\mcp.rs assumes when it is given no --server-url.

.EXAMPLE
    .\build.ps1 -Run
    Release build, then serve it on http://localhost:5151.

.EXAMPLE
    .\build.ps1 -Check
    What CI will do to your branch, before you push it.
#>
[CmdletBinding()]
param(
    [switch] $Dev,
    [switch] $Check,
    [switch] $SkipFrontend,
    [switch] $Run,
    [int]    $Port = 5151
)

$ErrorActionPreference = 'Stop'

$RepoRoot = $PSScriptRoot
# NOT $Profile -- that is a PowerShell automatic variable holding the path to
# the user's profile script, and shadowing it here would be a confusing thing
# to leave behind in anyone's session.
$BuildProfile = if ($Dev) { 'debug' } else { 'release' }
$OutDir       = Join-Path $RepoRoot "target\$BuildProfile"

function Invoke-Step {
    <#
        Native tools do not throw on failure -- $ErrorActionPreference has no
        say over an exit code -- so every step checks one explicitly. Without
        this the script would sail past a failed `npm run build` and hand you a
        binary serving the previous bundle, which is the exact drift it exists
        to prevent.
    #>
    param([string] $Label, [scriptblock] $Command)

    Write-Host "==> $Label" -ForegroundColor Cyan
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "$Label failed (exit $LASTEXITCODE)" }
}

Push-Location $RepoRoot
try {
    Write-Host "RoomMate -- $BuildProfile build in place" -ForegroundColor Cyan
    Write-Host ''

    # FIRST, and the order is load-bearing: `cargo test` regenerates the
    # TypeScript the settings page reads (ts-rs, into src-js\settings\generated\).
    # Typechecking before it would check the previous build's types and pass on
    # a settings field that no longer exists.
    if ($Check) { Invoke-Step 'cargo test' { cargo test } }

    # ---------------------------------------------------------- frontend ----
    if (-not $SkipFrontend) {
        if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
            throw 'npm not found. Install Node 24, or pass -SkipFrontend to build against the committed bundle.'
        }

        # `ci`, not `install`: it installs exactly what package-lock.json pins
        # and fails if the lockfile disagrees with package.json. Only when
        # node_modules is absent, because `ci` deletes and reinstalls the whole
        # tree every time it runs, and that is a minute nobody needs per build.
        if (-not (Test-Path (Join-Path $RepoRoot 'node_modules'))) {
            Invoke-Step 'npm ci' { npm ci }
        }

        if ($Check) {
            Invoke-Step 'npm run typecheck' { npm run typecheck }
            Invoke-Step 'npm test'          { npm test }
        }

        Invoke-Step 'npm run build' { npm run build }
    }
    elseif (-not (Test-Path (Join-Path $RepoRoot 'static\vendor\renderer.bundle.js'))) {
        throw 'No committed renderer bundle to skip to -- run without -SkipFrontend.'
    }

    # ----------------------------------------------------------- binaries ---
    # Both bins named explicitly. A bare `cargo build` builds them both today
    # anyway, but mcp.exe is the half that is easy to forget exists, and the
    # installer names both for the same reason.
    $cargoArgs = @('build', '--bin', 'roommate', '--bin', 'mcp')
    if (-not $Dev) { $cargoArgs += '--release' }
    Invoke-Step "cargo $($cargoArgs -join ' ')" { cargo @cargoArgs }

    if ($Check) {
        Invoke-Step 'cargo fmt --check' { cargo fmt --check }
        # clippy runs with -D warnings in CI, so a warning is a failure. Same
        # here, or -Check would pass on something the PR goes red for.
        Invoke-Step 'cargo clippy' { cargo clippy --all-targets -- -D warnings }
    }

    # ------------------------------------------------- generated artifacts --
    # A NOTE, never a failure. The rebuild legitimately changes these while you
    # are working in src-js\ -- what it must not do is reach a push unnoticed,
    # because frontend.yml rebuilds and diffs them and then goes red for a
    # reason that reads as unrelated to whatever you actually changed.
    if (-not $SkipFrontend) {
        $generated = git status --porcelain -- static/vendor/ static/settings/ src-js/settings/generated/
        if ($generated) {
            Write-Host ''
            Write-Host 'Generated files this build changed -- commit them or CI will go red:' -ForegroundColor Yellow
            $generated | ForEach-Object { Write-Host "  $_" -ForegroundColor Yellow }
        }
    }

    # -------------------------------------------------------------- report --
    $exe = Join-Path $OutDir 'roommate.exe'
    Write-Host ''
    Write-Host "Built $exe" -ForegroundColor Green

    # roommate.exe prefers a static\ directory BESIDE the executable over the
    # working directory, and build.rs refreshes that copy whenever it already
    # exists -- so it is current as of the cargo build above. Where it does not
    # exist (a fresh target\) the fallback is the working directory's static\,
    # which is why the run below sets the working directory rather than
    # assuming whatever the caller happened to be in.
    $runCmd = ".\target\$BuildProfile\roommate.exe --server-settings settings\server.toml --project-settings settings\projects --port $Port"

    if ($Run) {
        Write-Host ''
        Write-Host "Serving http://localhost:$Port -- Ctrl+C to stop" -ForegroundColor Cyan
        Write-Host ''
        & $exe --server-settings 'settings\server.toml' --project-settings 'settings\projects' --port $Port
    }
    else {
        Write-Host ''
        Write-Host 'Run it with:' -ForegroundColor Cyan
        Write-Host "  $runCmd"
    }
}
finally {
    Pop-Location
}
