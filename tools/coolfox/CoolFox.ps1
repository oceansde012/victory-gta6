# One-click runner for oceansde012/cool_fox (CGBJ, pareto-residency).
# First run: clones + builds, finds the game, copies our latest save into a
# separate data folder, makes a desktop shortcut. Later runs just launch.
# Touches nothing outside $Root and %LOCALAPPDATA%\VIctory\cache-coolfox.
param(
    [string] $Root = 'B:\cool_fox',
    [string] $GameDir = $env:VICTORY_GAME_DIR,
    [switch] $Rebuild,
    [switch] $FreshSave,     # re-copy the save from our builds
    [switch] $NoLaunch
)
$ErrorActionPreference = 'Stop'
$src  = Join-Path $Root 'src'
$data = Join-Path $Root 'data'
$logs = Join-Path $Root 'logs'
New-Item -ItemType Directory -Force $Root, $logs | Out-Null

# 1. Source + build
if (!(Test-Path "$src\.git")) {
    git clone https://github.com/oceansde012/cool_fox $src
    if ($LASTEXITCODE) { throw 'git clone failed' }
}
$exe = Join-Path $src 'target\release\victory.exe'
if ($Rebuild -or !(Test-Path $exe)) {
    Push-Location $src
    try {
        git pull --ff-only
        cargo build --release --locked -p victory 2>&1 | Tee-Object (Join-Path $logs 'build.log')
        if ($LASTEXITCODE) { throw "cargo build failed, see $logs\build.log" }
    } finally { Pop-Location }
}

# 2. Game folder (env > remembered > search B:)
$gameTxt = Join-Path $Root 'game-dir.txt'
if (!$GameDir -and (Test-Path $gameTxt)) { $GameDir = (Get-Content $gameTxt -Raw).Trim() }
if (!$GameDir -or !(Test-Path "$GameDir\game_scarlett_master.exe")) {
    $hit = Get-ChildItem 'B:\' -Recurse -Depth 4 -Filter game_scarlett_master.exe -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (!$hit) { throw 'game_scarlett_master.exe not found on B:\ (depth 4). Run with -GameDir <folder>.' }
    $GameDir = $hit.DirectoryName
}
Set-Content $gameTxt $GameDir -Encoding UTF8
Write-Host "Game: $GameDir"

# 3. Save: copy the newest title folder (saves + storage) from our builds' data dirs
if ($FreshSave -or !(Test-Path $data)) {
    $cands = @("$env:LOCALAPPDATA\victory\data", "$env:LOCALAPPDATA\DexNom Runtime") |
        Where-Object { Test-Path $_ } |
        ForEach-Object { Get-ChildItem $_ -Recurse -Depth 3 -Directory -Filter saves -ErrorAction SilentlyContinue } |
        Sort-Object LastWriteTime -Descending
    if ($cands) {
        $title = $cands[0].Parent          # <data>\<TITLEID>
        $dest  = Join-Path $data $title.Name
        if (Test-Path $dest) { Rename-Item $dest ("$($title.Name).old-" + (Get-Date -f yyyyMMdd-HHmmss)) }
        New-Item -ItemType Directory -Force $data | Out-Null
        Copy-Item $title.FullName $dest -Recurse
        Remove-Item (Join-Path $dest 'crashes.log') -ErrorAction SilentlyContinue
        Write-Host "Save copied from $($title.FullName) ($($cands[0].LastWriteTime))"
    } else {
        Write-Warning 'No save found in our data dirs; the game starts from scratch.'
        New-Item -ItemType Directory -Force $data | Out-Null
    }
}

# 4. Desktop shortcut (once)
$lnk = Join-Path ([Environment]::GetFolderPath('Desktop')) 'cool_fox (CGBJ).lnk'
$self = Join-Path $Root 'CoolFox.ps1'
if ($PSCommandPath -and $PSCommandPath -ne $self) { Copy-Item $PSCommandPath $self -Force }
if (!(Test-Path $lnk)) {
    $s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
    $s.TargetPath = 'powershell.exe'
    $s.Arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$self`""
    $s.WorkingDirectory = $Root
    $s.Save()
}
if ($NoLaunch) { Write-Host 'Ready.'; return }

# 5. Launch with the fork's own launcher (its pareto-residency env), our dirs
$env:VICTORY_GAME_DIR  = $GameDir
$env:VICTORY_DATA_DIR  = $data
$env:VICTORY_CACHE_DIR = "$env:LOCALAPPDATA\VIctory\cache-coolfox"
$stamp = Get-Date -f yyyyMMdd-HHmmss
Push-Location $src
try {
    cmd /c "Play-VIctory.cmd pareto-residency 2>&1" | Tee-Object (Join-Path $logs "run-$stamp.log")
} finally { Pop-Location }
