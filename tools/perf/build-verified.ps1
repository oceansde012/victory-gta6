param(
    [Parameter(Mandatory = $true)]
    [string]$ExpectedCommit,
    [string]$TargetDir
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if (-not $TargetDir) { $TargetDir = Join-Path $repo 'target' }
$target = [System.IO.Path]::GetFullPath($TargetDir)
Push-Location $repo
try {
    $head = (& git rev-parse --verify HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read repository HEAD.' }
    $expected = (& git rev-parse --verify "$ExpectedCommit^{commit}").Trim()
    if ($LASTEXITCODE -ne 0 -or $head -ne $expected) {
        throw "HEAD does not match the requested commit: $head"
    }
    $changes = @(git status --porcelain --untracked-files=all)
    if ($LASTEXITCODE -ne 0 -or $changes.Count -ne 0) {
        throw 'Working tree is not clean; build from a clean checkout.'
    }

    $oldTarget = $env:CARGO_TARGET_DIR
    try {
        $env:CARGO_TARGET_DIR = $target
        & cargo build --locked --release -p victory
        if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed.' }
    } finally {
        $env:CARGO_TARGET_DIR = $oldTarget
    }
    if ((& git rev-parse --verify HEAD).Trim() -ne $head -or @(git status --porcelain --untracked-files=all).Count -ne 0) {
        throw 'The checkout changed during the build; no manifest was saved.'
    }
    $exe = Join-Path $target 'release/victory.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build did not produce $exe" }
    $hash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
    $manifest = [ordered]@{
        commit = $head
        executable = $exe
        sha256 = $hash
        profile = 'release'
        rustc = (& rustc --version | Out-String).Trim()
        built_utc = [DateTime]::UtcNow.ToString('o')
    }
    $manifestPath = Join-Path $target 'victory-build-manifest.json'
    $manifest | ConvertTo-Json | Set-Content -LiteralPath $manifestPath -Encoding utf8
    Write-Host "Verified build manifest: $manifestPath"
    Write-Host "Executable SHA-256: $hash"
} finally {
    Pop-Location
}
