Set-StrictMode -Version Latest

# L5 recheck 2026-10-19: the release host was a console-subsystem program (PE
# Subsystem 3); started from Explorer it opened a console window whose closing
# ended the application. The packaged host must be a Windows GUI program.
function Get-PortableExecutableSubsystem {
    param([Parameter(Mandatory = $true)][string]$Path)
    $stream = [IO.File]::OpenRead($Path)
    try {
        $header = New-Object byte[] 4096
        $read = $stream.Read($header, 0, $header.Length)
    }
    finally {
        $stream.Dispose()
    }
    if ($read -lt 64 -or $header[0] -ne 0x4D -or $header[1] -ne 0x5A) {
        throw "Not a PE image (no MZ header): $Path"
    }
    $peOffset = [BitConverter]::ToInt32($header, 0x3C)
    # Optional header = signature (4) + COFF header (20); Subsystem is at offset
    # 68 of the optional header for both PE32 and PE32+.
    $subsystemOffset = $peOffset + 24 + 68
    if ($peOffset -lt 64 -or $subsystemOffset + 2 -gt $read -or
        $header[$peOffset] -ne 0x50 -or $header[$peOffset + 1] -ne 0x45 -or
        $header[$peOffset + 2] -ne 0 -or $header[$peOffset + 3] -ne 0) {
        throw "Not a PE image (no PE signature): $Path"
    }
    return [int][BitConverter]::ToUInt16($header, $subsystemOffset)
}

function Assert-WindowsGuiSubsystem {
    param([Parameter(Mandatory = $true)][string]$Path)
    $subsystem = Get-PortableExecutableSubsystem -Path $Path
    if ($subsystem -ne 2) {
        throw "The Tauri Host must be a Windows GUI program (PE Subsystem 2), found $subsystem in $Path"
    }
}

function Invoke-TauriHostComponentBuild {
    param([Parameter(Mandatory = $true)][string]$RepositoryRoot)
    $tauriCli = Join-Path $RepositoryRoot "ui\node_modules\.bin\tauri.cmd"
    if (-not (Test-Path -LiteralPath $tauriCli -PathType Leaf)) {
        throw "Tauri CLI not found: $tauriCli"
    }
    Push-Location $RepositoryRoot
    try {
        & $tauriCli build --no-bundle --config src-tauri\tauri.conf.json | Out-Host
        if ($LASTEXITCODE -ne 0) {
            throw "Tauri Host component build failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
    $hostArtifact = Join-Path $RepositoryRoot "target\release\eostage-desktop.exe"
    if (-not (Test-Path -LiteralPath $hostArtifact -PathType Leaf)) {
        throw "Tauri Host component seam did not produce: $hostArtifact"
    }
    Assert-WindowsGuiSubsystem -Path $hostArtifact
    return [pscustomobject]@{ host_path = $hostArtifact }
}

function Publish-TauriPortableLayout {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$OutputRoot,
        [Parameter(Mandatory = $true)][string]$HostPath,
        [Parameter(Mandatory = $true)][string]$WorkerPath,
        [Parameter(Mandatory = $true)][string]$WorkerManifestPath
    )
    foreach ($required in @($HostPath, $WorkerPath, $WorkerManifestPath)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw "Portable component input is missing: $required"
        }
    }
    if (-not (Test-Path -LiteralPath $OutputRoot -PathType Container)) {
        New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
    }
    $hostCopy = Join-Path $OutputRoot "EOStage.exe"
    $runtimeWorkerRoot = Join-Path $OutputRoot "runtime\compat-worker"
    $runtimeWorker = Join-Path $runtimeWorkerRoot "eostage-compat-worker.exe"
    $runtimeManifest = Join-Path $runtimeWorkerRoot "component-manifest.json"
    $resourcesRoot = Join-Path $OutputRoot "resources"
    $mapFallbackSource = Join-Path $RepositoryRoot "ui\public\map-fallback.svg"
    $mapFallback = Join-Path $resourcesRoot "map-fallback.svg"
    if (-not (Test-Path -LiteralPath $mapFallbackSource -PathType Leaf)) {
        throw "Portable map fallback source is missing: $mapFallbackSource"
    }
    foreach ($destination in @($hostCopy, $runtimeWorker, $runtimeManifest, $mapFallback)) {
        if (Test-Path -LiteralPath $destination) {
            throw "Refusing to overwrite an existing portable component: $destination"
        }
    }
    New-Item -ItemType Directory -Path $runtimeWorkerRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $resourcesRoot -Force | Out-Null
    Copy-Item -LiteralPath $HostPath -Destination $hostCopy
    Copy-Item -LiteralPath $WorkerPath -Destination $runtimeWorker
    Copy-Item -LiteralPath $WorkerManifestPath -Destination $runtimeManifest
    Copy-Item -LiteralPath $mapFallbackSource -Destination $mapFallback

    $manifest = Get-Content -LiteralPath $runtimeManifest -Raw | ConvertFrom-Json
    $workerHash = (Get-FileHash -LiteralPath $runtimeWorker -Algorithm SHA256).Hash.ToLowerInvariant()
    if ([string]$manifest.sha256 -ne $workerHash) {
        throw "Published Compatibility Worker does not match its component manifest."
    }
    return [pscustomobject]@{
        host_path = $hostCopy
        worker_path = $runtimeWorker
        worker_manifest_path = $runtimeManifest
        map_fallback_path = $mapFallback
        host_sha256 = (Get-FileHash -LiteralPath $hostCopy -Algorithm SHA256).Hash.ToLowerInvariant()
        worker_sha256 = $workerHash
        worker_manifest_sha256 = (Get-FileHash -LiteralPath $runtimeManifest -Algorithm SHA256).Hash.ToLowerInvariant()
        map_fallback_sha256 = (Get-FileHash -LiteralPath $mapFallback -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Get-TauriDeliveryFileRecord {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$RelativePath
    )
    $item = Get-Item -LiteralPath $Path
    return [ordered]@{
        path = $RelativePath.Replace('\', '/')
        bytes = [int64]$item.Length
        sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Write-TauriDeliveryComponentManifest {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$OutputRoot,
        [Parameter(Mandatory = $true)]$Portable,
        [Parameter(Mandatory = $true)]$NativeGisIdentity,
        [Parameter(Mandatory = $true)][string]$SourceCommit
    )
    $manifestPath = Join-Path $OutputRoot "components-manifest.json"
    if (Test-Path -LiteralPath $manifestPath) {
        throw "Refusing to overwrite existing component delivery manifest: $manifestPath"
    }
    $tauriConfig = Get-Content -LiteralPath (Join-Path $RepositoryRoot "src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
    $workerManifest = Get-Content -LiteralPath $Portable.worker_manifest_path -Raw | ConvertFrom-Json
    $runtimeRoot = Join-Path $OutputRoot "runtime\gis"
    $gridRoot = Join-Path $OutputRoot "runtime\vertical-grids"

    $runtimeBytes = [int64]0
    foreach ($relativePath in @($NativeGisIdentity.runtime_component.package_paths)) {
        $runtimeBytes += [int64](Get-Item -LiteralPath (Join-Path $runtimeRoot $relativePath)).Length
    }

    $components = [Collections.Generic.List[object]]::new()
    $components.Add([ordered]@{
        id = "eostage-core"
        kind = "application"
        version = [string]$tauriConfig.version
        install_root = "."
        required_for_startup = $true
        depends_on = @()
        artifacts = @(
            (Get-TauriDeliveryFileRecord -Path $Portable.host_path -RelativePath "EOStage.exe"),
            (Get-TauriDeliveryFileRecord -Path $Portable.map_fallback_path -RelativePath "resources/map-fallback.svg")
        )
    })
    $components.Add([ordered]@{
        id = "compat-worker"
        kind = "managed_runtime"
        version = [string]$workerManifest.version
        install_root = "runtime/compat-worker"
        required_for_startup = $false
        depends_on = @()
        protocol = $workerManifest.protocol
        artifacts = @(
            (Get-TauriDeliveryFileRecord -Path $Portable.worker_path -RelativePath "runtime/compat-worker/eostage-compat-worker.exe"),
            (Get-TauriDeliveryFileRecord -Path $Portable.worker_manifest_path -RelativePath "runtime/compat-worker/component-manifest.json")
        )
    })
    $components.Add([ordered]@{
        id = "native-gis-runtime"
        kind = "runtime"
        version = [string]$NativeGisIdentity.runtime_id
        install_root = "runtime/gis"
        required_for_startup = $false
        depends_on = @()
        manifest_sha256 = [string]$NativeGisIdentity.runtime_component.manifest_sha256
        files_verified = [int]$NativeGisIdentity.runtime_component.files_verified
        bytes = $runtimeBytes
    })
    $expectedGridIds = @("vertical-grid-egm96", "vertical-grid-egm2008")
    $actualGridIds = @($NativeGisIdentity.vertical_grid_component.grids |
        ForEach-Object { [string]$_.component_id } |
        Sort-Object)
    if (@(Compare-Object -ReferenceObject ($expectedGridIds | Sort-Object) -DifferenceObject $actualGridIds).Count -ne 0) {
        throw "Vertical-grid delivery identity must contain exactly EGM96 and EGM2008."
    }
    foreach ($grid in @($NativeGisIdentity.vertical_grid_component.grids)) {
        $gridPath = Join-Path $gridRoot $grid.relative_path
        $gridRecord = Get-TauriDeliveryFileRecord -Path $gridPath -RelativePath ("runtime/vertical-grids/" + [string]$grid.relative_path)
        if ($gridRecord.sha256 -ne [string]$grid.sha256 -or $gridRecord.bytes -ne [int64]$grid.bytes) {
            throw "Vertical-grid component identity changed after GIS staging: $($grid.component_id)"
        }
        $components.Add([ordered]@{
            id = [string]$grid.component_id
            kind = "scientific_resource"
            version = [string]$grid.version
            install_root = "runtime/vertical-grids"
            required_for_startup = $false
            depends_on = @("native-gis-runtime")
            model = [string]$grid.model
            grid_id = [string]$grid.grid_id
            license = [string]$grid.license
            coverage = [string]$grid.coverage
            resolution = [string]$grid.resolution
            artifact = $gridRecord
        })
    }

    $manifest = [ordered]@{
        schema_version = 1
        product = "EOStage"
        app_version = [string]$tauriConfig.version
        delivery_model = "tauri-component-layout-v1"
        source_commit = $SourceCommit
        distribution_sources = @()
        components = @($components.ToArray())
    }
    [IO.File]::WriteAllText(
        $manifestPath,
        ($manifest | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    return [pscustomobject]@{
        path = $manifestPath
        sha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        component_count = $components.Count
    }
}
