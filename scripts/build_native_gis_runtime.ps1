<#
.SYNOPSIS
    Stage the product-owned native GIS runtime without Python.

.DESCRIPTION
    This script accepts an already reviewed Windows x64 GDAL/PROJ runtime
    directory. It deliberately does not call Python, PyInstaller, pip, Conda,
    QGIS, OSGeo4W, or a network component manager. The source directory must
    contain only the native runtime that has already passed the project's
    version, driver, license, and redistribution review.

    The script fails closed when either required vertical grid or the explicit
    notice directory is absent. It emits the manifest consumed by
    crates/insar-gis and a separate vertical-grids/manifest.json consumed by
    VerticalGridRegistry.

.PARAMETER SourceRoot
    Reviewed native runtime root. Expected layout: bin, share/proj, share/gdal.

.PARAMETER OutputRoot
    Product-owned runtime/gis staging directory.

.PARAMETER Egm96Grid
    Reviewed EGM96 grid resource (the current product may use the existing
    egm96_15.npz representation until a reviewed native grid representation is
    selected).

.PARAMETER Egm2008Grid
    Reviewed EGM2008 grid resource. This is mandatory; AC2 must not silently
    fall back to EGM96.

.PARAMETER NoticeRoot
    Directory containing the exact redistribution notices for the selected
    native runtime and both grids.

.PARAMETER ProductionAdmitted
    Explicit review decision. Without this switch the staging manifest records
    both grids as not production-admitted; the switch must only be used after
    science, provenance, and redistribution review are complete.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceRoot,
    [string]$OutputRoot = "",
    [Parameter(Mandatory = $true)]
    [string]$Egm96Grid,
    [Parameter(Mandatory = $true)]
    [string]$Egm2008Grid,
    [Parameter(Mandatory = $true)]
    [string]$NoticeRoot,
    [string]$GdalLibrary = "bin/gdal.dll",
    [string]$ProjLibrary = "bin/proj.dll",
    [string]$GdalVersion = "",
    [string]$ProjVersion = "",
    [string]$RuntimeId = "",
    [Parameter(Mandatory = $true)]
    [string]$SourceHorizontalCrs,
    [Parameter(Mandatory = $true)]
    [string]$SourceVerticalCrs,
    [Parameter(Mandatory = $true)]
    [string]$TargetHorizontalCrs,
    [Parameter(Mandatory = $true)]
    [string]$Egm96TargetVerticalCrs,
    [Parameter(Mandatory = $true)]
    [string]$Egm2008TargetVerticalCrs,
    [string]$Egm96Source = "US NGA-derived GeoTIFF converted from egm96_15.gtx; provenance bound by us_nga_README.txt",
    [string]$Egm96License = "Public Domain",
    [Parameter(Mandatory = $true)]
    [string]$Egm2008Source,
    [Parameter(Mandatory = $true)]
    [string]$Egm2008License,
    [string]$Egm96Version = "",
    [string]$Egm2008Version = "",
    [Parameter(Mandatory = $true)]
    [string]$Egm96Coverage,
    [Parameter(Mandatory = $true)]
    [string]$Egm2008Coverage,
    [Parameter(Mandatory = $true)]
    [string]$Egm96Resolution,
    [Parameter(Mandatory = $true)]
    [string]$Egm2008Resolution,
    [Parameter(Mandatory = $true)]
    [switch]$ProductionAdmitted
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

function Relative-ManifestPath {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$Path)
    # Windows PowerShell 5 does not expose .NET Core's Path.GetRelativePath.
    # URI.MakeRelativeUri is available on the supported Windows toolchain and
    # preserves spaces/non-ASCII names after explicit unescaping.
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $pathFull = [IO.Path]::GetFullPath($Path)
    $rootUri = New-Object System.Uri(($rootFull + [IO.Path]::DirectorySeparatorChar))
    $pathUri = New-Object System.Uri($pathFull)
    return [Uri]::UnescapeDataString($rootUri.MakeRelativeUri($pathUri).ToString()).Replace('\', '/')
}

function File-Manifest {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$Path)
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    $item = Get-Item -LiteralPath $Path
    return [ordered]@{
        relative_path = Relative-ManifestPath -Root $Root -Path $Path
        bytes = [int64]$item.Length
        sha256 = $hash
    }
}

function Grid-Manifest {
    param(
        [Parameter(Mandatory = $true)][string]$GridId,
        [Parameter(Mandatory = $true)][string]$Model,
        [Parameter(Mandatory = $true)][string]$SourcePath,
        [Parameter(Mandatory = $true)][string]$TargetPath,
        [Parameter(Mandatory = $true)][string]$Version,
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$License,
        [Parameter(Mandatory = $true)][string]$TargetVerticalCrs,
        [Parameter(Mandatory = $true)][string]$Coverage,
        [Parameter(Mandatory = $true)][string]$Resolution
    )
    $item = Get-Item -LiteralPath $SourcePath
    return [ordered]@{
        grid_id = $GridId
        model = $Model
        vertical_crs = $TargetVerticalCrs
        target_vertical_crs = $TargetVerticalCrs
        source_horizontal_crs = $SourceHorizontalCrs
        source_vertical_crs = $SourceVerticalCrs
        target_horizontal_crs = $TargetHorizontalCrs
        direction = "bidirectional"
        filename = Relative-ManifestPath -Root (Join-Path $OutputRoot "vertical-grids") -Path $TargetPath
        version = $Version
        source = $Source
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $TargetPath).Hash.ToLowerInvariant()
        expected_bytes = [int64]$item.Length
        coverage = $Coverage
        resolution = $Resolution
        license = $License
        bundled = $true
        production_admitted = [bool]$ProductionAdmitted.IsPresent
        applicable_transformations = @(
            "WGS84_ELLIPSOID_TO_${Model}",
            "${Model}_TO_WGS84_ELLIPSOID"
        )
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutputRoot) {
    $OutputRoot = Join-Path $repoRoot "dist\runtime\gis"
}
$SourceRoot = (Resolve-Path -LiteralPath $SourceRoot).Path
$Egm96Grid = (Resolve-Path -LiteralPath $Egm96Grid).Path
$Egm2008Grid = (Resolve-Path -LiteralPath $Egm2008Grid).Path
$NoticeRoot = (Resolve-Path -LiteralPath $NoticeRoot).Path
$OutputRoot = [IO.Path]::GetFullPath($OutputRoot)
$outputTrimmed = $OutputRoot.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
$outputLeaf = [IO.Path]::GetFileName($outputTrimmed)
$outputParentLeaf = [IO.Path]::GetFileName([IO.Path]::GetDirectoryName($outputTrimmed))
if ($outputLeaf -ne "gis" -or $outputParentLeaf -ne "runtime") {
    throw "OutputRoot must be a dedicated ...\runtime\gis directory; refusing to delete an ambiguous target: $OutputRoot"
}
if ($OutputRoot.Equals($SourceRoot, [StringComparison]::OrdinalIgnoreCase) -or
    $SourceRoot.StartsWith($OutputRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
    $OutputRoot.StartsWith($SourceRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "OutputRoot and the native source root must be disjoint"
}

Require-Directory -Path $SourceRoot -Label "reviewed native GIS source root"
Require-Directory -Path (Join-Path $SourceRoot "bin") -Label "native GIS bin directory"
Require-Directory -Path (Join-Path $SourceRoot "share\proj") -Label "native PROJ data directory"
Require-Directory -Path (Join-Path $SourceRoot "share\gdal") -Label "native GDAL data directory"
Require-File -Path (Join-Path $SourceRoot $GdalLibrary) -Label "GDAL library"
Require-File -Path (Join-Path $SourceRoot $ProjLibrary) -Label "PROJ library"
Require-File -Path $Egm96Grid -Label "EGM96 grid"
Require-File -Path $Egm2008Grid -Label "EGM2008 grid"
Require-Directory -Path $NoticeRoot -Label "native GIS notice directory"
$provenancePath = Join-Path $NoticeRoot "BINARY-PROVENANCE.json"
$noticePath = Join-Path $NoticeRoot "NOTICE-NATIVE-GIS.md"
Require-File -Path $provenancePath -Label "native GIS binary provenance"
Require-File -Path $noticePath -Label "native GIS notice"
$provenance = Get-Content -LiteralPath $provenancePath -Raw | ConvertFrom-Json
if ($provenance.kind -ne "isolated-native-import-closure-audit" -or
    $provenance.architecture -ne "Windows x64") {
    throw "BINARY-PROVENANCE.json is not an isolated Windows x64 import-closure audit"
}
$sourceDllNames = @(Get-ChildItem -LiteralPath (Join-Path $SourceRoot "bin") -File -Force |
    Where-Object { $_.Extension -ieq ".dll" } |
    ForEach-Object { $_.Name.ToLowerInvariant() } |
    Sort-Object -Unique)
$provenanceDllNames = @($provenance.dlls |
    ForEach-Object { $_.filename.ToLowerInvariant() } |
    Sort-Object -Unique)
if ((Compare-Object -ReferenceObject $sourceDllNames -DifferenceObject $provenanceDllNames).Count -ne 0) {
    throw "BINARY-PROVENANCE.json DLL entries do not exactly match the staged native bin closure"
}
if (-not $provenance.license_files_copied -or @($provenance.license_files_copied).Count -eq 0) {
    throw "BINARY-PROVENANCE.json has no exact package license inventory"
}
foreach ($licenseRelativePath in @($provenance.license_files_copied)) {
    Require-File -Path (Join-Path $NoticeRoot $licenseRelativePath) -Label "native GIS package license"
}
foreach ($dll in @($provenance.dlls)) {
    if ([string]::IsNullOrWhiteSpace($dll.package.license) -or
        [string]::IsNullOrWhiteSpace($dll.package.url) -or
        [string]::IsNullOrWhiteSpace($dll.package.sha256)) {
        throw "BINARY-PROVENANCE.json contains a DLL without package URL, archive hash, or license: $($dll.filename)"
    }
}

$forbidden = Get-ChildItem -LiteralPath $SourceRoot -Recurse -File -Force |
    Where-Object { $_.Extension.ToLowerInvariant() -in @(".py", ".pyc", ".pyo", ".pyd", ".exe") -or $_.Name -match "^(python|python3)(\.exe)?$" }
if ($forbidden) {
    throw "native GIS source contains Python/PyInstaller or executable payloads; refusing to stage it: $($forbidden[0].FullName)"
}

# GDAL plugin DLLs are an authority expansion: a source package that carries
# gdal_*.dll or a gdalplugins directory can silently re-enable HDF/netCDF/PDF/
# database/vector drivers. GTiff and ENVI must be supplied by the reviewed
# core GDAL build and are the only drivers named in the product manifest.
$pluginDirectories = Get-ChildItem -LiteralPath $SourceRoot -Recurse -Directory -Force |
    Where-Object { $_.Name -ieq "gdalplugins" -or $_.Name -ieq "gdalplugins-external" }
if ($pluginDirectories) {
    throw "native GIS source contains a GDAL plugin directory; refusing to stage an unbounded driver closure: $($pluginDirectories[0].FullName)"
}
$pluginFiles = Get-ChildItem -LiteralPath $SourceRoot -Recurse -File -Force |
    Where-Object { $_.Name -match "^gdal_.*\.dll$" }
if ($pluginFiles) {
    throw "native GIS source contains GDAL plugin DLLs; refusing to stage an unbounded driver closure: $($pluginFiles[0].FullName)"
}

if ([string]::IsNullOrWhiteSpace($GdalVersion) -or [string]::IsNullOrWhiteSpace($ProjVersion) -or [string]::IsNullOrWhiteSpace($RuntimeId)) {
    throw "GdalVersion, ProjVersion, and RuntimeId are mandatory review inputs"
}
if ([string]::IsNullOrWhiteSpace($Egm96Version) -or [string]::IsNullOrWhiteSpace($Egm2008Version)) {
    throw "Egm96Version and Egm2008Version are mandatory review inputs"
}
if ([string]::IsNullOrWhiteSpace($Egm96Coverage) -or [string]::IsNullOrWhiteSpace($Egm2008Coverage) -or
    [string]::IsNullOrWhiteSpace($Egm96Resolution) -or [string]::IsNullOrWhiteSpace($Egm2008Resolution)) {
    throw "grid coverage and resolution are mandatory review inputs"
}

if (Test-Path -LiteralPath $OutputRoot) {
    Remove-Item -LiteralPath $OutputRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $OutputRoot "bin") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $OutputRoot "share") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $OutputRoot "vertical-grids\egm96") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $OutputRoot "vertical-grids\egm2008") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $OutputRoot "licenses") | Out-Null

Copy-Item -LiteralPath (Get-ChildItem -LiteralPath (Join-Path $SourceRoot "bin") -File -Force).FullName -Destination (Join-Path $OutputRoot "bin") -Force
Copy-Item -LiteralPath (Join-Path $SourceRoot "share\proj") -Destination (Join-Path $OutputRoot "share") -Recurse -Force
Copy-Item -LiteralPath (Join-Path $SourceRoot "share\gdal") -Destination (Join-Path $OutputRoot "share") -Recurse -Force
Copy-Item -LiteralPath (Get-ChildItem -LiteralPath $NoticeRoot -Force).FullName -Destination (Join-Path $OutputRoot "licenses") -Recurse -Force

$egm96Target = Join-Path $OutputRoot "vertical-grids\egm96\$([IO.Path]::GetFileName($Egm96Grid))"
$egm2008Target = Join-Path $OutputRoot "vertical-grids\egm2008\$([IO.Path]::GetFileName($Egm2008Grid))"
Copy-Item -LiteralPath $Egm96Grid -Destination $egm96Target -Force
Copy-Item -LiteralPath $Egm2008Grid -Destination $egm2008Target -Force

$egm96Entry = Grid-Manifest -GridId "egm96-global" -Model "EGM96" -SourcePath $Egm96Grid -TargetPath $egm96Target -Version $Egm96Version -Source $Egm96Source -License $Egm96License -TargetVerticalCrs $Egm96TargetVerticalCrs -Coverage $Egm96Coverage -Resolution $Egm96Resolution
$egm2008Entry = Grid-Manifest -GridId "egm2008-global" -Model "EGM2008" -SourcePath $Egm2008Grid -TargetPath $egm2008Target -Version $Egm2008Version -Source $Egm2008Source -License $Egm2008License -TargetVerticalCrs $Egm2008TargetVerticalCrs -Coverage $Egm2008Coverage -Resolution $Egm2008Resolution

$verticalManifest = @($egm96Entry, $egm2008Entry) | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText((Join-Path $OutputRoot "vertical-grids\manifest.json"), $verticalManifest, [Text.UTF8Encoding]::new($false))

$files = Get-ChildItem -LiteralPath $OutputRoot -Recurse -File -Force |
    Where-Object { $_.FullName -ne (Join-Path $OutputRoot "manifest.json") } |
    Sort-Object FullName |
    ForEach-Object { File-Manifest -Root $OutputRoot -Path $_.FullName }

$manifest = [ordered]@{
    schema_version = 1
    runtime_id = $RuntimeId
    gdal_version = $GdalVersion
    proj_version = $ProjVersion
    gdal_library = $GdalLibrary.Replace('\', '/')
    proj_library = $ProjLibrary.Replace('\', '/')
    proj_data_dir = "share/proj"
    gdal_data_dir = "share/gdal"
    proj_network = "disabled"
    drivers = @("GTiff", "ENVI")
    files = @($files)
}
[IO.File]::WriteAllText((Join-Path $OutputRoot "manifest.json"), ($manifest | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))

Write-Host "Native GIS runtime staged at $OutputRoot" -ForegroundColor Green
Write-Host "GDAL $GdalVersion / PROJ $ProjVersion; bounded raster drivers: GTiff, ENVI"
Write-Host "EGM96: $((Get-FileHash -Algorithm SHA256 -LiteralPath $egm96Target).Hash.ToLowerInvariant()) ($((Get-Item -LiteralPath $egm96Target).Length) bytes)"
Write-Host "EGM2008: $((Get-FileHash -Algorithm SHA256 -LiteralPath $egm2008Target).Hash.ToLowerInvariant()) ($((Get-Item -LiteralPath $egm2008Target).Length) bytes)"
