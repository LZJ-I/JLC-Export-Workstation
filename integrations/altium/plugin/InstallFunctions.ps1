# Functions separated so discovery, registration and rollback can be tested in
# isolated folders without an installed AD or touching its real configuration.
function Get-LcedaInstallations {
    $found = @()
    foreach ($key in @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*')) {
        foreach ($entry in @(Get-ItemProperty $key -ErrorAction SilentlyContinue)) {
            if ($entry.PSObject.Properties['DisplayName'] -and $entry.DisplayName -match '^Altium Designer' -and $entry.PSObject.Properties['InstallLocation'] -and $entry.InstallLocation -and (Test-Path -LiteralPath $entry.InstallLocation)) {
                $found += [IO.Path]::GetFullPath($entry.InstallLocation)
            }
        }
    }
    return @($found | Sort-Object -Unique)
}
function Get-LcedaExtensionRoots {
    $base = Join-Path $env:ProgramData 'Altium'
    if (-not (Test-Path -LiteralPath $base)) { return @() }
    return @(Get-ChildItem -LiteralPath $base -Directory | ForEach-Object { Join-Path $_.FullName 'Extensions' } | Where-Object { Test-Path -LiteralPath (Join-Path $_ 'ExtensionsRegistry.xml') })
}
function Find-LcedaSdk([string]$AdDirectory, [string]$ExtensionsDirectory) {
    $matches = @()
    foreach ($root in @($AdDirectory, (Join-Path $ExtensionsDirectory 'AltiumDeveloper'))) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        foreach ($file in @(Get-ChildItem -LiteralPath $root -Filter 'Altium.SDK.dll' -File -Recurse -ErrorAction SilentlyContinue)) {
            $interfaces = Join-Path $file.DirectoryName 'Altium.SDK.Interfaces.dll'
            if (Test-Path -LiteralPath $interfaces) { $matches += $file.DirectoryName }
        }
    }
    $matches = @($matches | Sort-Object -Unique)
    if ($matches.Count -ne 1) {
        throw "Expected one matching AD SDK folder; found $($matches.Count). Install the Altium Developer SDK for this AD version, or select an AD directory containing its Altium.SDK.dll and Altium.SDK.Interfaces.dll. No library files were changed."
    }
    return $matches[0]
}
function Build-LcedaPlugin([string]$SdkDirectory, [string]$SourceDirectory, [string]$OutputDirectory) {
    $compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
    if (-not (Test-Path -LiteralPath $compiler)) { throw 'The Windows .NET Framework C# compiler is unavailable. Install .NET Framework 4.8.' }
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $dll = Join-Path $OutputDirectory 'Lceda.Altium.dll'
    $arguments = @('/nologo', '/target:library', '/platform:x64', '/optimize+', '/debug-', '/warn:4', "/out:$dll", '/reference:System.dll', '/reference:System.Core.dll', '/reference:System.Windows.Forms.dll', "/reference:$(Join-Path $SdkDirectory 'Altium.SDK.dll')", "/reference:$(Join-Path $SdkDirectory 'Altium.SDK.Interfaces.dll')")
    foreach ($name in @('Host.cs', 'BridgeRuntime.cs', 'AdEditor.cs')) { $arguments += Join-Path $SourceDirectory $name }
    $log = @(& $compiler @arguments 2>&1)
    [IO.File]::WriteAllLines((Join-Path $OutputDirectory 'build.log'), [string[]]$log)
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $dll)) {
        throw "Plugin compilation against this AD version failed. No installed plugin or library was changed. Details: $(Join-Path $OutputDirectory 'build.log')`n$($log -join "`n")"
    }
    foreach ($name in @('Lceda.Altium.Ins', 'Lceda.Altium.rcs', 'README_ZH.md')) { Copy-Item -LiteralPath (Join-Path $SourceDirectory $name) -Destination (Join-Path $OutputDirectory $name) }
    return $dll
}
function New-LcedaRegistry([string]$RegistryPath, [string]$DeployDirectory, [bool]$Remove) {
    $xml = New-Object Xml.XmlDocument
    $xml.PreserveWhitespace = $true
    $xml.Load($RegistryPath)
    if ($xml.DocumentElement.Name -ne 'Extensions') { throw 'Unsupported ExtensionsRegistry.xml root. The original registry is unchanged.' }
    $items = @($xml.SelectNodes('/Extensions/Item[@HRID="Lceda.Altium"]'))
    if ($items.Count -gt 1) { throw 'Duplicate Lceda.Altium extension entries. Review ExtensionsRegistry.xml before installing.' }
    if ($Remove) { foreach ($item in $items) { $xml.DocumentElement.RemoveChild($item) | Out-Null }; return $xml }
    if ($items.Count -eq 1) { $item = $items[0] }
    else {
        $item = $xml.CreateElement('Item'); $item.SetAttribute('HRID', 'Lceda.Altium'); $item.SetAttribute('Guid', '026080A0-BB31-4090-9CD5-A325B61836CC')
        $xml.DocumentElement.AppendChild($item) | Out-Null
    }
    $ole = [DateTime]::UtcNow.ToOADate().ToString('F7', [Globalization.CultureInfo]::InvariantCulture)
    $fields = [ordered]@{ Path = $DeployDirectory; Status = '0'; VaultGuid = ''; CreatedBy = 'LZJ-I'; CategoryGuid = '793A1F67-0B22-4E01-A5DE-3176A1E8C60D'; CategoryName = ''; ReadMe = ''; Help = ''; Requirements = ''; Title = 'JLC Export Workstation'; ShortDescription = 'Native library synchronization'; LongDescription = 'Append new library parts from JLC Export Workstation inside AD'; SmallImage = ''; LargeImage = ''; Version = '0.9.0.0'; VersionGuid = '040C40AB-928D-4DD2-B137-9D508D96EE6C'; ReleasedDate = $ole; ReleaseNotes = ''; DateInstalled = $ole }
    foreach ($key in $fields.Keys) {
        $node = $item.SelectSingleNode($key)
        if (-not $node) { $node = $xml.CreateElement($key); $item.AppendChild($node) | Out-Null }
        $node.InnerText = [string]$fields[$key]
    }
    if (-not $item.SelectSingleNode('PlatformVersions')) {
        $pv = $xml.CreateElement('PlatformVersions')
        foreach ($name in @('DXP', 'EDP', 'MaxDXP', 'MaxEDP')) { $node = $xml.CreateElement($name); $node.SetAttribute('BuildNumber', $(if ($name -like 'Max*') { '0.0.0.0' } else { '1.0.16.41' })); $pv.AppendChild($node) | Out-Null }
        $item.AppendChild($pv) | Out-Null
    }
    return $xml
}
function Save-LcedaXml([Xml.XmlDocument]$Xml, [string]$Path) {
    $settings = New-Object Xml.XmlWriterSettings
    $settings.Encoding = New-Object Text.UTF8Encoding($false)
    $writer = [Xml.XmlWriter]::Create($Path, $settings)
    try { $Xml.Save($writer) } finally { $writer.Dispose() }
    # Reparse before the atomic swap to reject truncated/invalid output.
    $check = New-Object Xml.XmlDocument; $check.Load($Path)
}
function Invoke-LcedaInstall([string]$Action, [string]$AdDirectory, [string]$ExtensionsDirectory, [string]$BridgeDirectory, [string]$SourceDirectory) {
    if ($Action -eq 'Discover') { return @{ ok = $true; installations = @(Get-LcedaInstallations); extensions = @(Get-LcedaExtensionRoots) } }
    if (Get-Process -Name 'X2', 'DXP', 'AltiumDesigner' -ErrorAction SilentlyContinue) { throw 'Close Altium Designer before installing or removing the native plugin, then restart AD after installation. No running documents were touched.' }
    if ($Action -eq 'Uninstall' -and (Test-Path -LiteralPath (Join-Path $BridgeDirectory 'native-install.json') -PathType Leaf)) {
        $installed = Get-Content -LiteralPath (Join-Path $BridgeDirectory 'native-install.json') -Encoding UTF8 -Raw | ConvertFrom-Json
        if (-not $AdDirectory -and $installed.PSObject.Properties['ad_directory']) { $AdDirectory = $installed.ad_directory }
        if (-not $ExtensionsDirectory -and $installed.PSObject.Properties['extensions_directory']) { $ExtensionsDirectory = $installed.extensions_directory }
    }
    if (-not $AdDirectory) {
        $installs = @(Get-LcedaInstallations)
        if ($installs.Count -ne 1) { throw "Found $($installs.Count) Altium Designer installations. Select the AD installation directory in the workstation connection window." }
        $AdDirectory = $installs[0]
    }
    if (-not (Test-Path -LiteralPath (Join-Path $AdDirectory 'X2.exe'))) { throw 'The selected directory does not contain Altium Designer X2.exe.' }
    if (-not $ExtensionsDirectory) {
        $roots = @(Get-LcedaExtensionRoots)
        # Never guess the most recently modified root when versions coexist.
        if ($roots.Count -ne 1) { throw "Found $($roots.Count) AD extension registries. Select the Extensions directory for this AD version." }
        $ExtensionsDirectory = $roots[0]
    }
    $registry = Join-Path $ExtensionsDirectory 'ExtensionsRegistry.xml'
    if (-not (Test-Path -LiteralPath $registry)) { throw 'ExtensionsRegistry.xml is missing from the selected AD Extensions directory.' }
    New-Item -ItemType Directory -Path $BridgeDirectory -Force | Out-Null
    $stamp = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss-fffffff')
    $work = Join-Path $BridgeDirectory "native-build-$stamp"
    $deploy = Join-Path $ExtensionsDirectory 'Lceda.Altium'
    $backup = Join-Path $ExtensionsDirectory "Lceda.Altium-backup-$stamp"
    $xmlBackup = "$registry.lceda-backup-$stamp"
    $xmlNew = "$registry.lceda-$stamp.tmp"
    $stage = Join-Path $ExtensionsDirectory "Lceda.Altium-stage-$stamp"
    $movedOld = $false; $movedNew = $false; $replaced = $false
    $profileBefore = @{}
    foreach ($name in @('native-install.json', 'enabled', 'stop', 'file-mode')) {
        $path = Join-Path $BridgeDirectory $name
        $profileBefore[$name] = @{ file = (Test-Path -LiteralPath $path -PathType Leaf); directory = (Test-Path -LiteralPath $path -PathType Container); bytes = $null }
        if ($profileBefore[$name].file) { $profileBefore[$name].bytes = [IO.File]::ReadAllBytes($path) }
    }
    # Fully build and validate the new registry before changing AD's installation.
    if ($Action -eq 'Install') { $sdk = Find-LcedaSdk $AdDirectory $ExtensionsDirectory; Build-LcedaPlugin $sdk $SourceDirectory $work | Out-Null }
    $xml = New-LcedaRegistry $registry $deploy ($Action -eq 'Uninstall')
    Save-LcedaXml $xml $xmlNew
    try {
        Copy-Item -LiteralPath $registry -Destination $xmlBackup
        if ($Action -eq 'Install') { Copy-Item -LiteralPath $work -Destination $stage -Recurse }
        if (Test-Path -LiteralPath $deploy) { Move-Item -LiteralPath $deploy -Destination $backup; $movedOld = $true }
        if ($Action -eq 'Install') { Move-Item -LiteralPath $stage -Destination $deploy; $movedNew = $true }
        [IO.File]::Replace($xmlNew, $registry, [NullString]::Value); $replaced = $true
        $state = @{ ok = $true; version = '0.9.0'; ad_directory = $AdDirectory; extensions_directory = $ExtensionsDirectory; plugin = $deploy; registry_backup = $xmlBackup; installed = ($Action -eq 'Install'); message = $(if ($Action -eq 'Install') { 'Native plugin installed. Start AD to connect automatically.' } else { 'Native plugin removed. Bound libraries were retained.' }) }
        $statePath = Join-Path $BridgeDirectory 'native-install.json'
        [IO.File]::WriteAllText($statePath, ($state | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false)))
        if ($Action -eq 'Install') {
            [IO.File]::WriteAllText((Join-Path $BridgeDirectory 'enabled'), '1')
            if (Test-Path -LiteralPath (Join-Path $BridgeDirectory 'stop')) { Remove-Item -LiteralPath (Join-Path $BridgeDirectory 'stop') }
            if (Test-Path -LiteralPath (Join-Path $BridgeDirectory 'file-mode')) { Remove-Item -LiteralPath (Join-Path $BridgeDirectory 'file-mode') }
        }
        return $state
    } catch {
        # Restore only this plugin and the extension registry. User libraries are
        # outside this transaction and are never read or rewritten by installation.
        if ($replaced) { Copy-Item -LiteralPath $xmlBackup -Destination $registry -Force }
        if ($movedNew -and (Test-Path -LiteralPath $deploy)) { Remove-Item -LiteralPath $deploy -Recurse -Force }
        if ($movedOld -and (Test-Path -LiteralPath $backup)) { Move-Item -LiteralPath $backup -Destination $deploy }
        foreach ($name in $profileBefore.Keys) {
            $path = Join-Path $BridgeDirectory $name; $previous = $profileBefore[$name]
            if ($previous.file) { [IO.File]::WriteAllBytes($path, $previous.bytes) }
            elseif (-not $previous.directory -and (Test-Path -LiteralPath $path -PathType Leaf)) { Remove-Item -LiteralPath $path }
        }
        throw
    } finally {
        if (Test-Path -LiteralPath $xmlNew) { Remove-Item -LiteralPath $xmlNew }
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    }
}
