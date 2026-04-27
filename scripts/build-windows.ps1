param(
    [switch]$Release,
    [switch]$SkipLint,
    [switch]$StrictLint
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$AppDir = Join-Path $ProjectRoot "crates\fastclaw-app"
$TauriDir = Join-Path $AppDir "src-tauri"
$DistDir = Join-Path $ProjectRoot "dist"
$KeyPath = Join-Path $env:USERPROFILE ".tauri\fastclaw.key"

function Log($msg) { Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Err($msg) { Write-Host "[ERR ] $msg" -ForegroundColor Red }
function Ok($msg) { Write-Host "[ OK ] $msg" -ForegroundColor Green }

Log "Checking build environment..."
foreach ($cmd in @("cargo", "corepack", "node")) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        Err "Missing command: $cmd"
        exit 1
    }
}

if (-not (Test-Path $KeyPath)) {
    Err "Signing key not found: $KeyPath"
    Write-Host "  Generate it with:"
    Write-Host ('  npx @tauri-apps/cli@latest signer generate --write-keys "' + $KeyPath + '" --force -p ""')
    exit 1
}

$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $KeyPath -Raw
if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
}

$TauriConf = Get-Content (Join-Path $TauriDir "tauri.conf.json") | ConvertFrom-Json
$Version = $TauriConf.version
Ok "Version: v$Version"
Ok "Cargo: $(cargo --version)"
Ok "Node: $(node --version)"
Ok "pnpm: $(corepack pnpm --version)"

if (-not $SkipLint) {
    if ($StrictLint) {
        Log "Running clippy (strict: -D warnings)..."
    } else {
        Log "Running clippy (non-strict)..."
    }
    Push-Location $ProjectRoot
    if ($StrictLint) {
        cargo clippy --workspace --all-targets -j 1 -- -D warnings
    } else {
        cargo clippy --workspace --all-targets -j 1
    }
    if ($LASTEXITCODE -ne 0) {
        Pop-Location
        Err "Clippy failed"
        exit 1
    }
    Pop-Location
    Ok "Clippy passed"
}

Log "Installing frontend dependencies..."
Push-Location $AppDir
corepack pnpm install --frozen-lockfile
if ($LASTEXITCODE -ne 0) {
    Pop-Location
    Err "pnpm install failed"
    exit 1
}

Log "Building frontend..."
corepack pnpm build
if ($LASTEXITCODE -ne 0) {
    Pop-Location
    Err "Frontend build failed"
    exit 1
}
Pop-Location
Ok "Frontend build complete"

Log "Building Tauri app (Windows)..."
Push-Location $AppDir
corepack pnpm exec tauri build
if ($LASTEXITCODE -ne 0) {
    Pop-Location
    Err "Tauri build failed"
    exit 1
}
Pop-Location
Ok "Tauri build complete"

Log "Collecting build artifacts..."
if (Test-Path $DistDir) {
    Remove-Item -Recurse -Force $DistDir
}
New-Item -ItemType Directory -Force -Path $DistDir | Out-Null

$BundleDir = Join-Path $ProjectRoot "target\tauri\release\bundle"
$Patterns = @("*.exe", "*.msi", "*.nsis.zip", "*.nsis.zip.sig")
foreach ($pattern in $Patterns) {
    Get-ChildItem -Recurse -Path $BundleDir -Filter $pattern -ErrorAction SilentlyContinue |
        Copy-Item -Destination $DistDir
}

Ok "Artifacts collected at $DistDir"
Get-ChildItem $DistDir | Select-Object Name, Length | Format-Table -AutoSize

if ($Release) {
    Log "Generating latest.json..."
    $NsisZip = Get-ChildItem $DistDir -Filter "*.nsis.zip" |
        Where-Object { $_.Name -notmatch "\.sig$" } |
        Select-Object -First 1
    $NsisSig = Get-ChildItem $DistDir -Filter "*.nsis.zip.sig" | Select-Object -First 1

    if (-not $NsisZip -or -not $NsisSig) {
        Err "NSIS zip or signature file not found"
        exit 1
    }

    $SigContent = Get-Content $NsisSig.FullName -Raw
    $PubDate = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    $LatestJson = @{
        version = $Version
        notes = "FastClaw v$Version"
        pub_date = $PubDate
        platforms = @{
            "windows-x86_64" = @{
                url = "REPLACE_WITH_DOWNLOAD_URL/$($NsisZip.Name)"
                signature = $SigContent.Trim()
            }
        }
    } | ConvertTo-Json -Depth 4

    $LatestJson | Out-File -FilePath (Join-Path $DistDir "latest.json") -Encoding utf8
    Ok "latest.json generated"
    Write-Host "  Edit $DistDir\\latest.json and replace the url field"
    Write-Host "  Replace REPLACE_WITH_DOWNLOAD_URL with the real download URL"
}

Write-Host ""
Ok "Windows build complete. Artifacts: $DistDir"
Write-Host "  Artifact list:"
Get-ChildItem $DistDir | ForEach-Object {
    $size = ($_.Length / 1MB).ToString("N2") + " MB"
    Write-Host "    $($_.Name)  ($size)"
}

if ($Release) {
    Write-Host ""
    Write-Host "  Release steps:"
    Write-Host "    1. Upload all files in dist"
    Write-Host "    2. Update latest.json url to real download URL"
    Write-Host "    3. Host latest.json on your update endpoint"
}
