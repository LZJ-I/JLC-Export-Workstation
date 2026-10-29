param([string]$TestBinary,[string]$AppBinary,[string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
$taskRoot = Join-Path $env:TEMP ('lceda-updater-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $taskRoot,$EvidenceDirectory -Force | Out-Null
$runner = Join-Path $taskRoot 'updater-tests.exe'
$candidate = Join-Path $taskRoot 'candidate.exe'
Copy-Item -LiteralPath $TestBinary -Destination $runner
Copy-Item -LiteralPath $AppBinary -Destination $candidate
$env:LCEDA_UPDATE_TEST_BINARY = $candidate
$env:LCEDA_UPDATE_EVIDENCE = $EvidenceDirectory
foreach ($test in @(@('unit','update:: --test-threads=4'), @('roundtrip','--exact update::runtime::tests::windows_update_roundtrip --ignored --test-threads=1 --nocapture'))) {
    $stdout = Join-Path $EvidenceDirectory ($test[0] + '.stdout.txt')
    $stderr = Join-Path $EvidenceDirectory ($test[0] + '.stderr.txt')
    $process = Start-Process -FilePath $runner -ArgumentList $test[1] -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $null = $process.Handle
    if (-not $process.WaitForExit(180000)) { $process.Kill(); throw ('Test timed out: ' + $test[0]) }
    $process.Refresh()
    Get-Content -LiteralPath $stdout -Encoding UTF8
    Get-Content -LiteralPath $stderr -Encoding UTF8
    if ($process.ExitCode -ne 0) { throw ('Failed test: ' + $test[0] + ' exit ' + $process.ExitCode) }
}
Write-Output ('PASS: native Windows updater tests; evidence: ' + $EvidenceDirectory)
