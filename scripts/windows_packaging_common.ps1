function Resolve-InnoSetupCompiler {
    param([string]$RequestedPath = "")

    if (-not [string]::IsNullOrWhiteSpace($RequestedPath)) {
        if (-not (Test-Path -LiteralPath $RequestedPath -PathType Leaf)) {
            throw "Requested Inno Setup compiler does not exist: $RequestedPath"
        }
        return (Resolve-Path -LiteralPath $RequestedPath).Path
    }

    $command = Get-Command "ISCC.exe" -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($command) {
        return (Resolve-Path -LiteralPath $command.Source).Path
    }

    $candidates = @()
    foreach ($registryPath in @(
        "Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\App Paths\ISCC.exe",
        "Registry::HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\App Paths\ISCC.exe",
        "Registry::HKEY_LOCAL_MACHINE\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\ISCC.exe"
    )) {
        if (Test-Path -LiteralPath $registryPath) {
            $registered = (Get-Item -LiteralPath $registryPath).GetValue("")
            if (-not [string]::IsNullOrWhiteSpace($registered)) {
                $candidates += [string]$registered
            }
        }
    }

    if (-not [string]::IsNullOrWhiteSpace(${env:ProgramFiles(x86)})) {
        $candidates += Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"
    }
    if (-not [string]::IsNullOrWhiteSpace($env:ProgramFiles)) {
        $candidates += Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe"
    }
    if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        $candidates += Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"
    }

    foreach ($candidate in $candidates) {
        $normalized = ([string]$candidate).Trim().Trim('"')
        if (Test-Path -LiteralPath $normalized -PathType Leaf) {
            return (Resolve-Path -LiteralPath $normalized).Path
        }
    }

    throw (
        "Inno Setup 6 compiler (ISCC.exe) was not found in PATH, App Paths, " +
        "Program Files, or the per-user Programs directory. Install Inno Setup 6 " +
        "or pass -IsccPath with the full compiler path."
    )
}

function Invoke-BoundedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string]$Arguments = "",
        [ValidateRange(1, 3600)][int]$TimeoutSeconds = 300,
        [string]$Description = "Process",
        [string]$WorkingDirectory = ""
    )

    $startArguments = @{
        FilePath = $FilePath
        PassThru = $true
        WindowStyle = "Hidden"
    }
    if (-not [string]::IsNullOrWhiteSpace($Arguments)) {
        $startArguments.ArgumentList = $Arguments
    }
    if (-not [string]::IsNullOrWhiteSpace($WorkingDirectory)) {
        $startArguments.WorkingDirectory = $WorkingDirectory
    }

    $process = Start-Process @startArguments
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "$Description timed out after $TimeoutSeconds seconds (PID $($process.Id))"
    }
    $process.Refresh()
    return $process
}
