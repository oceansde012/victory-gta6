# Run with PowerShell. Uses local Git fixtures and simulates Cargo output.
$ErrorActionPreference = 'Stop'
$runner = Join-Path $PSScriptRoot 'CoolFox.ps1'
$gitCommand = (Get-Command git -CommandType Application | Select-Object -First 1).Source
$fixtureName = '.coolfox-test-' + [guid]::NewGuid().ToString('N')
$fixture = Join-Path $PSScriptRoot $fixtureName
$desktopShortcut = Join-Path ([Environment]::GetFolderPath('Desktop')) 'cool_fox (CGBJ).lnk'
$builds = [System.Collections.Generic.List[object]]::new()
$previousTarget = $env:CARGO_TARGET_DIR

function Assert-CoolFox([bool] $Condition, [string] $Message) {
    if (!$Condition) { throw $Message }
}

function Test-Path {
    param([string] $Path, [string] $LiteralPath, [string] $PathType)
    if ($Path -eq $desktopShortcut) { return $true }
    Microsoft.PowerShell.Management\Test-Path @PSBoundParameters
}

function git {
    if ($args[0] -ne 'pull') { throw "Unexpected Git call from CoolFox: $args" }
    & $gitCommand -c core.hooksPath=NUL @args
}

function cmd {
    if ($args[1] -notlike 'cargo build *') { throw "Unexpected cmd call from CoolFox: $args" }
    $builds.Add([pscustomobject]@{ Checkout = (Get-Location).Path; Target = $env:CARGO_TARGET_DIR })
    $release = Join-Path $env:CARGO_TARGET_DIR 'release'
    New-Item -ItemType Directory -Force $release | Out-Null
    Set-Content -LiteralPath (Join-Path $release 'victory.exe') -Value 'fixture only'
    Set-Variable -Name LASTEXITCODE -Value 0 -Scope 1
    'Simulated Cargo build'
}

function Invoke-FixtureGit {
    & $gitCommand -c core.hooksPath=NUL -c user.name='CoolFox Tests' -c user.email='test@example.invalid' @args
    if ($LASTEXITCODE) { throw "Fixture Git command failed: $args" }
}

try {
    $root = Join-Path $fixture 'CoolFox workspace'
    $seed = Join-Path $fixture 'seed'
    $game = Join-Path $fixture 'game'
    $variant = Join-Path $root 'picks'
    $primary = Join-Path $root 'src'
    New-Item -ItemType Directory -Force $root, $seed, $game, (Join-Path $root 'data') | Out-Null
    Set-Content -LiteralPath (Join-Path $game 'game_scarlett_master.exe') -Value 'fixture only'
    $options = @{ Root = $root; GameDir = $game; NoLaunch = $true }

    Invoke-FixtureGit init --quiet -b main $seed
    Invoke-FixtureGit -C $seed commit --quiet --allow-empty -m 'Initial fixture'
    Invoke-FixtureGit -C $seed worktree add --quiet --detach $variant HEAD
    $variantHead = Invoke-FixtureGit -C $variant rev-parse HEAD
    $env:CARGO_TARGET_DIR = Join-Path $fixture 'ambient-cargo-target'

    & $runner @options -Variant picks
    Assert-CoolFox ($builds.Count -eq 1) 'Missing variant executable should trigger one build.'
    Assert-CoolFox ($builds[0].Checkout -eq $variant) 'Variant should build in its own checkout.'
    Assert-CoolFox ($builds[0].Target -eq (Join-Path $variant 'target')) 'Variant should build into its own target.'
    Assert-CoolFox (!(Test-Path -LiteralPath $primary)) 'Variant should not create src.'
    Assert-CoolFox (Test-Path -LiteralPath (Join-Path $root 'logs\build-picks.log')) 'Variant build log is missing.'
    Assert-CoolFox ($env:CARGO_TARGET_DIR -eq (Join-Path $fixture 'ambient-cargo-target')) 'Ambient Cargo target was not restored.'

    & $runner @options -Variant picks
    Assert-CoolFox ($builds.Count -eq 1) 'Cached variant should skip the build.'
    Invoke-FixtureGit -C $seed commit --quiet --allow-empty -m 'Advance the source'
    & $runner @options -Variant picks -Rebuild
    Assert-CoolFox ($builds.Count -eq 2 -and $builds[1].Checkout -eq $variant) 'Variant rebuild should stay in its checkout.'
    Assert-CoolFox ((Invoke-FixtureGit -C $variant rev-parse HEAD) -eq $variantHead) 'Variant rebuild should preserve its pinned commit.'

    Invoke-FixtureGit clone --quiet $seed $primary
    Invoke-FixtureGit -C $seed commit --quiet --allow-empty -m 'Update the default checkout'
    & $runner @options
    Assert-CoolFox ($builds.Count -eq 3 -and $builds[2].Checkout -eq $primary) 'Default build should run in src.'
    Assert-CoolFox ($builds[2].Target -eq (Join-Path $primary 'target')) 'Default checkout should build into src\target.'
    Assert-CoolFox ((Invoke-FixtureGit -C $primary rev-parse HEAD) -eq (Invoke-FixtureGit -C $seed rev-parse HEAD)) 'Default checkout should fast-forward before building.'

    Invoke-FixtureGit -C $primary commit --quiet --allow-empty -m 'Local-only change'
    Invoke-FixtureGit -C $seed commit --quiet --allow-empty -m 'Remote-only change'
    $primaryHead = Invoke-FixtureGit -C $primary rev-parse HEAD
    $pullFailure = $null
    try { & $runner @options -Rebuild } catch { $pullFailure = $_ }
    Assert-CoolFox ($pullFailure.Exception.Message -like 'git pull --ff-only failed in *') 'A divergent pull should be reported.'
    Assert-CoolFox ((Invoke-FixtureGit -C $primary rev-parse HEAD) -eq $primaryHead) 'A divergent pull should preserve the local commit.'
    Assert-CoolFox ($builds.Count -eq 3) 'Cargo ran after a failed pull.'
    Write-Host 'CoolFox regression tests passed.'
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    $resolvedFixture = Resolve-Path -LiteralPath $fixture -ErrorAction SilentlyContinue
    if ($resolvedFixture -and $resolvedFixture.Path -eq (Join-Path $PSScriptRoot $fixtureName)) {
        Remove-Item -LiteralPath $resolvedFixture.Path -Recurse -Force
    }
}
