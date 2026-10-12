#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('Install', 'Discover', 'Uninstall')][string]$Action = 'Install',
    [string]$AdDirectory,
    [string]$ExtensionsDirectory,
    [string]$BridgeDirectory,
    [string]$ResultPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'InstallFunctions.ps1')

try {
    if (-not $BridgeDirectory) {
        $BridgeDirectory = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'LZJ-I\lceda-assistant\config\ad-bridge'
    }
    $result = Invoke-LcedaInstall -Action $Action -AdDirectory $AdDirectory -ExtensionsDirectory $ExtensionsDirectory -BridgeDirectory $BridgeDirectory -SourceDirectory $PSScriptRoot
    $json = $result | ConvertTo-Json -Depth 6
    if ($ResultPath) { [IO.File]::WriteAllText($ResultPath, $json, (New-Object Text.UTF8Encoding($false))) }
    Write-Output $json
    exit 0
} catch {
    $message = $_.Exception.Message
    $json = @{ ok = $false; message = $message } | ConvertTo-Json
    if ($ResultPath) { [IO.File]::WriteAllText($ResultPath, $json, (New-Object Text.UTF8Encoding($false))) }
    Write-Output $json
    exit 1
}
