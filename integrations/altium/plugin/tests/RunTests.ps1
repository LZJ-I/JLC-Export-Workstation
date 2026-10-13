#Requires -Version 5.1
param([string]$OutputDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $env:TEMP ("lceda-native-tests-" + [Guid]::NewGuid().ToString('N')) }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$source = Split-Path $PSScriptRoot -Parent
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
. (Join-Path $source 'InstallFunctions.ps1')
function Assert([bool]$Condition, [string]$Name) { if (-not $Condition) { throw "FAIL $Name" }; Write-Output "PASS $Name" }
function Compile([string[]]$Arguments) { $output = @(& $compiler /nologo @Arguments 2>&1); if ($LASTEXITCODE -ne 0) { throw ($output -join "`n") } }
$runtime = Join-Path $OutputDirectory 'RuntimeTests.exe'
Compile @('/target:exe', "/out:$runtime", '/reference:System.Core.dll', (Join-Path $source 'BridgeRuntime.cs'), (Join-Path $PSScriptRoot 'RuntimeTests.cs'))
& $runtime $OutputDirectory
if ($LASTEXITCODE -ne 0) { throw 'Native runtime tests failed' }

$ad = Join-Path $OutputDirectory 'AD-fixture'; $sdk = Join-Path $ad 'SDK'; $ext = Join-Path $OutputDirectory 'Extensions'; $bridge = Join-Path $OutputDirectory 'profile'
New-Item -ItemType Directory -Path $sdk, $ext, $bridge -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $ad 'X2.exe'), 'fixture, not executable')
Compile @('/target:library', "/out:$(Join-Path $sdk 'Altium.SDK.dll')", (Join-Path $PSScriptRoot 'HostContract.cs'))
$empty = Join-Path $OutputDirectory 'EmptyInterfaces.cs'; [IO.File]::WriteAllText($empty, 'internal sealed class TestInterfaces { }')
Compile @('/target:library', "/out:$(Join-Path $sdk 'Altium.SDK.Interfaces.dll')", $empty)
$contractTest = Join-Path $sdk 'EditorContractTests.exe'
Compile @('/target:exe', "/out:$contractTest", '/reference:System.Core.dll', "/reference:$(Join-Path $sdk 'Altium.SDK.dll')", (Join-Path $source 'AdEditor.cs'), (Join-Path $source 'BridgeRuntime.cs'), (Join-Path $PSScriptRoot 'EditorContractTests.cs'))
& $contractTest $OutputDirectory
if ($LASTEXITCODE -ne 0) { throw 'Unsupported editor contract test failed' }
$registry = Join-Path $ext 'ExtensionsRegistry.xml'
[IO.File]::WriteAllText($registry, '<Extensions><Item HRID="Existing.Plugin" Guid="keep-me"><Path>unchanged</Path><Status>0</Status><Custom>preserve</Custom></Item></Extensions>')
$original = [IO.File]::ReadAllBytes($registry)
[IO.File]::WriteAllText((Join-Path $bridge 'file-mode'), 'explicit file mode before installation')
$result = Invoke-LcedaInstall 'Install' $ad $ext $bridge $source
Assert ($result.ok -and (Test-Path (Join-Path $ext 'Lceda.Altium\Lceda.Altium.dll'))) 'factory and editor code compile using test host contract; isolated install succeeds'
Assert (-not (Test-Path (Join-Path $bridge 'file-mode'))) 'native installation selects editor mode explicitly'
[xml]$xml = Get-Content -LiteralPath $registry
Assert ($xml.SelectSingleNode('/Extensions/Item[@HRID="Existing.Plugin"]/Custom').InnerText -eq 'preserve') 'other AD extension configuration retained'
Assert ($xml.SelectNodes('/Extensions/Item[@HRID="Lceda.Altium"]').Count -eq 1) 'own extension registered exactly once'
Assert (@(Get-ChildItem -LiteralPath (Join-Path $ext 'Lceda.Altium') -Filter 'Altium.SDK*.dll').Count -eq 0) 'commercial SDK and test SDK binaries are not deployed'
$result = Invoke-LcedaInstall 'Install' $ad $ext $bridge $source
[xml]$xml = Get-Content -LiteralPath $registry
Assert ($xml.SelectNodes('/Extensions/Item[@HRID="Lceda.Altium"]').Count -eq 1) 'reinstall is idempotent'
Assert (@(Get-ChildItem -LiteralPath $ext -Directory -Filter 'Lceda.Altium-backup-*').Count -eq 1) 'previous plugin retained as a backup'
$before = [IO.File]::ReadAllBytes($registry)
$failed = $false; try { Invoke-LcedaInstall 'Install' $ad (Join-Path $OutputDirectory 'missing-registry') $bridge $source } catch { $failed = $true }
Assert ($failed -and [Convert]::ToBase64String($before) -eq [Convert]::ToBase64String([IO.File]::ReadAllBytes($registry))) 'missing registry does not modify the installation'
$dllHash = (Get-FileHash (Join-Path $ext 'Lceda.Altium\Lceda.Altium.dll')).Hash
$statePath = Join-Path $bridge 'native-install.json'; $state = [IO.File]::ReadAllBytes($statePath)
Remove-Item -LiteralPath $statePath; New-Item -ItemType Directory -Path $statePath | Out-Null
$failed = $false; try { Invoke-LcedaInstall 'Install' $ad $ext $bridge $source } catch { $failed = $true }
Assert ($failed -and (Get-FileHash (Join-Path $ext 'Lceda.Altium\Lceda.Altium.dll')).Hash -eq $dllHash -and [Convert]::ToBase64String($before) -eq [Convert]::ToBase64String([IO.File]::ReadAllBytes($registry))) 'failed final installation step restores registry and plugin'
Remove-Item -LiteralPath $statePath; [IO.File]::WriteAllBytes($statePath, $state)
$enabledPath = Join-Path $bridge 'enabled'; Remove-Item -LiteralPath $enabledPath; New-Item -ItemType Directory -Path $enabledPath | Out-Null
[IO.File]::WriteAllText((Join-Path $bridge 'file-mode'), 'retain-file-mode-on-failure')
$failed = $false; try { Invoke-LcedaInstall 'Install' $ad $ext $bridge $source } catch { $failed = $true }
Assert ($failed -and [Convert]::ToBase64String($state) -eq [Convert]::ToBase64String([IO.File]::ReadAllBytes($statePath))) 'failed enable step also restores per-user installation state'
Assert ((Get-Content -LiteralPath (Join-Path $bridge 'file-mode') -Raw) -eq 'retain-file-mode-on-failure') 'failed installation retains explicit file mode'
Remove-Item -LiteralPath $enabledPath
$result = Invoke-LcedaInstall 'Uninstall' '' '' $bridge $source
[xml]$xml = Get-Content -LiteralPath $registry
Assert (-not $result.installed -and $xml.SelectNodes('/Extensions/Item[@HRID="Lceda.Altium"]').Count -eq 0 -and $xml.SelectNodes('/Extensions/Item[@HRID="Existing.Plugin"]').Count -eq 1) 'uninstall removes only own entry and retains old plugin backup'
Assert (Test-Path (Join-Path $bridge 'file-mode')) 'uninstall preserves an explicit file mode choice'
$missingSdk = Join-Path $ad 'no-sdk'; New-Item -ItemType Directory -Path $missingSdk | Out-Null
$failed = $false; try { Find-LcedaSdk $missingSdk $ext } catch { $failed = $true }
Assert $failed 'missing real SDK fails clearly'
Write-Output "All plugin checks passed. Native AD loading/editing was NOT tested. Evidence: $OutputDirectory"
