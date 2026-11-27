Set-StrictMode -Version Latest

function Assert-NativeGisPathChain {
    param([Parameter(Mandatory = $true)][string]$Path)
    $fullPath = [IO.Path]::GetFullPath($Path)
    $cursor = $fullPath
    while (-not [string]::IsNullOrEmpty($cursor)) {
        $item = $null
        try { $item = Get-Item -LiteralPath $cursor -Force -ErrorAction Stop }
        catch [System.Management.Automation.ItemNotFoundException] { }
        if ($null -ne $item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Native GIS delivery refuses a linked path or ancestor."
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
    return $fullPath
}

function Get-NativeGisDeliveryIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$RuntimeRoot,
        [string]$VerticalGridRoot = ""
    )
    $arguments = @("--runtime-root", $RuntimeRoot)
    if (-not [string]::IsNullOrWhiteSpace($VerticalGridRoot)) {
        $arguments += @("--vertical-grid-root", $VerticalGridRoot)
    }
    $identityJson = & node (Join-Path $RepositoryRoot "scripts\native_gis_package_contract.mjs") @arguments
    if ($LASTEXITCODE -ne 0) { throw "Native GIS runtime failed its declared file and path validation." }
    $identity = ($identityJson -join "`n") | ConvertFrom-Json
    if ($identity.ok -ne $true -or @($identity.package_paths).Count -eq 0) {
        throw "Native GIS identity validation did not return a bounded copy plan."
    }
    return $identity
}

function Assert-NativeGisDeliveryInputs {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [string]$GisRuntimeRoot,
        [string]$NativeDemSmokeInput
    )
    if ([string]::IsNullOrWhiteSpace($GisRuntimeRoot)) {
        throw "Production DEM requires -GisRuntimeRoot. A compatibility Worker is not the production runtime/gis dependency."
    }
    if ([string]::IsNullOrWhiteSpace($NativeDemSmokeInput) -or
        -not (Test-Path -LiteralPath $NativeDemSmokeInput -PathType Leaf)) {
        throw "Provide -NativeDemSmokeInput: a small fixed DEM fixture for actual GDAL/PROJ and SARscape verification."
    }
    $checker = Join-Path $RepositoryRoot "scripts\native_gis_package_contract.mjs"
    $identityJson = & node $checker --runtime-root $GisRuntimeRoot
    if ($LASTEXITCODE -ne 0) { throw "The production native GIS runtime did not pass artifact identity validation." }
    $identity = ($identityJson -join "`n") | ConvertFrom-Json
    if ($identity.ok -ne $true) { throw "Native GIS artifact identity validation did not report success." }
    return $identity
}

function Publish-NativeGisRuntime {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$GisRuntimeRoot,
        [Parameter(Mandatory = $true)][string]$OutputRoot
    )
    $sourceRoot = Assert-NativeGisPathChain -Path $GisRuntimeRoot
    $OutputRoot = Assert-NativeGisPathChain -Path $OutputRoot
    $destination = Assert-NativeGisPathChain -Path (Join-Path $OutputRoot "runtime\gis")
    $verticalDestination = Assert-NativeGisPathChain -Path (Join-Path $OutputRoot "runtime\vertical-grids")
    $readmePath = Join-Path $OutputRoot "README-FOLDER-PORTABLE.txt"
    if (Test-Path -LiteralPath $destination) {
        throw "Refusing to overwrite an existing native GIS runtime: $destination"
    }
    if (Test-Path -LiteralPath $verticalDestination) {
        throw "Refusing to overwrite existing vertical-grid resources: $verticalDestination"
    }
    if (Test-Path -LiteralPath $readmePath) { throw "Refusing to overwrite an existing portable README." }
    $sourcePrefix = $sourceRoot.TrimEnd('\') + '\'
    $destinationPrefix = $destination.TrimEnd('\') + '\'
    if ($sourcePrefix.StartsWith($destinationPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        $destinationPrefix.StartsWith($sourcePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Native GIS source and destination must not overlap."
    }
    # Recheck immediately before copying: this helper can be called without the builder preflight.
    $sourceIdentity = Get-NativeGisDeliveryIdentity -RepositoryRoot $RepositoryRoot -RuntimeRoot $sourceRoot
    foreach ($relativeDirectory in @($sourceIdentity.package_directories)) {
        $targetDirectory = Assert-NativeGisPathChain -Path (Join-Path $destination $relativeDirectory)
        New-Item -ItemType Directory -Path $targetDirectory -Force | Out-Null
    }
    foreach ($relativePath in @($sourceIdentity.runtime_component.package_paths)) {
        $source = Assert-NativeGisPathChain -Path (Join-Path $sourceRoot $relativePath)
        $target = Assert-NativeGisPathChain -Path (Join-Path $destination $relativePath)
        New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($target)) -Force | Out-Null
        [IO.File]::Copy($source, $target, $false)
    }
    foreach ($relativePath in @($sourceIdentity.vertical_grid_component.package_paths)) {
        $source = Assert-NativeGisPathChain -Path (Join-Path (Join-Path $sourceRoot "vertical-grids") $relativePath)
        $target = Assert-NativeGisPathChain -Path (Join-Path $verticalDestination $relativePath)
        New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($target)) -Force | Out-Null
        [IO.File]::Copy($source, $target, $false)
    }
    $identity = Get-NativeGisDeliveryIdentity `
        -RepositoryRoot $RepositoryRoot `
        -RuntimeRoot $destination `
        -VerticalGridRoot $verticalDestination
    if ($identity.manifest_sha256 -ne $sourceIdentity.manifest_sha256 -or
        $identity.vertical_manifest_sha256 -ne $sourceIdentity.vertical_manifest_sha256) {
        throw "Staged native GIS manifest identities changed while copying."
    }
    $readme = "EOStage folder portable package`r`n`r`nApplication core, native GIS runtime, and vertical-grid resources have separate delivery identities. Keep EOStage.exe beside runtime/gis and runtime/vertical-grids for complete DEM capability. The application core can start without optional GIS resources, but DEM conversion requiring GDAL/PROJ or EGM grids will fail closed. Do not substitute system GDAL, Python, Conda or QGIS.`r`n"
    $stream = [IO.File]::Open($readmePath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($readme)
        $stream.Write($bytes, 0, $bytes.Length)
    } finally { $stream.Dispose() }
    return $identity
}
