[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$HostExe,
    [Parameter(Mandatory = $true)]
    [string]$GisRuntimeRoot,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot,
    [Parameter(Mandatory = $true)]
    [string]$SourceCommit,
    [string]$ProductExeName = "EOStage.exe"
)

$ErrorActionPreference = "Stop"

function Require-File {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Label)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label is missing: $Path"
    }
}

function Require-Directory {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Label)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "$Label is missing: $Path"
    }
}

function Relative-Path {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$Path)
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $pathFull = [IO.Path]::GetFullPath($Path)
    $rootUri = New-Object System.Uri(($rootFull + [IO.Path]::DirectorySeparatorChar))
    $pathUri = New-Object System.Uri($pathFull)
    return [Uri]::UnescapeDataString($rootUri.MakeRelativeUri($pathUri).ToString()).Replace('\', '/')
}

function Sha256-Hex {
    param([Parameter(Mandatory = $true)][string]$Path)
    $stream = [IO.File]::OpenRead($Path)
    try {
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace("-", "").ToLowerInvariant()
        } finally {
            $sha.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

function File-Record {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$Path)
    $item = Get-Item -LiteralPath $Path
    return [ordered]@{
        relative_path = Relative-Path -Root $Root -Path $Path
        bytes = [int64]$item.Length
        sha256 = Sha256-Hex -Path $Path
    }
}

$HostExe = (Resolve-Path -LiteralPath $HostExe).Path
$GisRuntimeRoot = (Resolve-Path -LiteralPath $GisRuntimeRoot).Path
$OutputRoot = [IO.Path]::GetFullPath($OutputRoot)
$SourceCommit = $SourceCommit.ToLowerInvariant()
if ($SourceCommit -notmatch '^[0-9a-f]{40}$') {
    throw "SourceCommit must be exactly 40 hexadecimal characters"
}
$runtimeManifestPath = Join-Path $GisRuntimeRoot "manifest.json"
$verticalManifestPath = Join-Path $GisRuntimeRoot "vertical-grids\manifest.json"

Require-File -Path $HostExe -Label "Tauri host executable"
Require-Directory -Path $GisRuntimeRoot -Label "product-owned native GIS runtime"
Require-File -Path $runtimeManifestPath -Label "native GIS runtime manifest"
Require-File -Path $verticalManifestPath -Label "native vertical-grid manifest"

$hostItem = Get-Item -LiteralPath $HostExe
if ($hostItem.Extension -ine ".exe") {
    throw "Tauri host must be a Windows executable: $HostExe"
}
$runtimeManifest = Get-Content -LiteralPath $runtimeManifestPath -Raw | ConvertFrom-Json
if ($runtimeManifest.proj_network -ne "disabled") {
    throw "native GIS runtime must declare proj_network=disabled"
}
$drivers = @($runtimeManifest.drivers | ForEach-Object { [string]$_ }) | Sort-Object
if (($drivers -join ",") -ne "ENVI,GTiff") {
    throw "native GIS runtime driver capability must be exactly GTiff and ENVI"
}

if (Test-Path -LiteralPath $OutputRoot) {
    throw "portable output already exists; use a fresh isolated staging directory: $OutputRoot"
}
New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $OutputRoot "runtime") -Force | Out-Null
Copy-Item -LiteralPath $HostExe -Destination (Join-Path $OutputRoot $ProductExeName) -Force
Copy-Item -LiteralPath $GisRuntimeRoot -Destination (Join-Path $OutputRoot "runtime\gis") -Recurse -Force

$portableReadme = @"
EOStage folder portable package

This directory is the distributable unit. "$ProductExeName" is NOT a
self-contained portable executable and must remain beside runtime/gis.

Required layout:
  $ProductExeName
  runtime/gis/

The package does not require or discover system Python, GDAL, PROJ, Conda,
QGIS, or OSGeo4W. Do not copy the EXE by itself.
"@
[IO.File]::WriteAllText(
    (Join-Path $OutputRoot "README-FOLDER-PORTABLE.txt"),
    $portableReadme,
    [Text.UTF8Encoding]::new($false)
)

$forbidden = Get-ChildItem -LiteralPath $OutputRoot -Recurse -File -Force |
    Where-Object {
        $_.Name -match "(?i)(pyinstaller|compat-worker|python|conda|qgis|osgeo4w)" -or
        $_.Extension.ToLowerInvariant() -in @(".py", ".pyc", ".pyo", ".pyd", ".spec")
    }
if ($forbidden) {
    throw "native Tauri portable staging contains a forbidden legacy/runtime payload: $($forbidden[0].FullName)"
}

$executables = @(Get-ChildItem -LiteralPath $OutputRoot -Recurse -File -Force |
    Where-Object { $_.Extension -ieq ".exe" })
if ($executables.Count -ne 1 -or $executables[0].Name -ne $ProductExeName) {
    throw "native Tauri portable staging must contain exactly one host executable"
}

$files = Get-ChildItem -LiteralPath $OutputRoot -Recurse -File -Force |
    Sort-Object FullName |
    ForEach-Object { File-Record -Root $OutputRoot -Path $_.FullName }
$payload = [ordered]@{
    schema_version = 1
    source_commit = $SourceCommit
    bundle_kind = "folder_portable"
    architecture = "Windows x64"
    host_is_self_contained = $false
    distributable_unit = "complete_directory"
    required_adjacent_runtime = "runtime/gis"
    package_readme = "README-FOLDER-PORTABLE.txt"
    host = File-Record -Root $OutputRoot -Path (Join-Path $OutputRoot $ProductExeName)
    runtime_manifest = "runtime/gis/manifest.json"
    vertical_grid_manifest = "runtime/gis/vertical-grids/manifest.json"
    files = @($files)
}
[IO.File]::WriteAllText(
    (Join-Path $OutputRoot "portable-manifest.json"),
    ($payload | ConvertTo-Json -Depth 12),
    [Text.UTF8Encoding]::new($false)
)

Write-Host "Native Tauri portable package staged at $OutputRoot" -ForegroundColor Green
Write-Host "Host: $ProductExeName ($($hostItem.Length) bytes)"
Write-Host "Delivery mode: folder portable; the EXE alone is not self-contained."
Write-Host "Runtime: $($runtimeManifest.runtime_id); GTiff + ENVI; PROJ network disabled"
Write-Host "No Python, Conda, QGIS, OSGeo4W, or compatibility component was staged."
