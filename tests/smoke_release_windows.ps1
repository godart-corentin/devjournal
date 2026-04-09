param(
    [Parameter(Mandatory = $true)]
    [string]$ArchivePath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$Message) {
    throw "FAIL: $Message"
}

function Assert-Contains([string]$Text, [string]$Needle, [string]$Label) {
    if (-not $Text.Contains($Needle)) {
        Write-Error $Text
        Fail $Label
    }
}

if (-not (Test-Path -LiteralPath $ArchivePath)) {
    Fail "Archive not found: $ArchivePath"
}

$workRoot = Join-Path $env:TEMP ("devjournal-smoke-" + [guid]::NewGuid().ToString("N"))
$installDir = Join-Path $workRoot "bin"
$repoDir = Join-Path $workRoot "repo"
$homeDir = Join-Path $workRoot "home"
$appDataDir = Join-Path $workRoot "appdata"
$localAppDataDir = Join-Path $workRoot "localappdata"
$binPath = $null

New-Item -ItemType Directory -Force -Path $installDir, $repoDir, $homeDir, $appDataDir, $localAppDataDir | Out-Null

$env:HOME = $homeDir
$env:USERPROFILE = $homeDir
$env:APPDATA = $appDataDir
$env:LOCALAPPDATA = $localAppDataDir

try {
    Expand-Archive -LiteralPath $ArchivePath -DestinationPath $installDir -Force
    $binPath = Join-Path $installDir "devjournal.exe"
    if (-not (Test-Path -LiteralPath $binPath)) {
        Fail "Extracted archive does not contain devjournal.exe"
    }

    Push-Location $repoDir
    try {
        git init -b main | Out-Null
        git config user.name "Smoke Tester"
        git config user.email "smoke@example.com"
        Set-Content -LiteralPath (Join-Path $repoDir "README.md") -Value "# smoke repo"
        git add README.md
        git commit -m "smoke commit" | Out-Null
    }
    finally {
        Pop-Location
    }

    $configPath = (& $binPath config).Trim()
    $configDir = Split-Path -Parent $configPath
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null

    $repoTomlPath = $repoDir.Replace("'", "''")
    @"
[general]
poll_interval_secs = 1
author = "Smoke Tester"

[llm]
provider = "cursor"
model = "gpt-5.4-mini"

[[repos]]
path = '$repoTomlPath'
name = "smoke-repo"
"@ | Set-Content -LiteralPath $configPath

    & $binPath start | Out-Null
    Start-Sleep -Seconds 2
    $runningStatus = (& $binPath status) | Out-String
    Assert-Contains $runningStatus "devjournal daemon: running" "daemon did not report running after start"

    & $binPath stop | Out-Null
    $stoppedStatus = (& $binPath status) | Out-String
    Assert-Contains $stoppedStatus "devjournal daemon: not running" "daemon did not report stopped after stop"

    $syncOutput = (& $binPath sync 2>&1 | Out-String)
    Assert-Contains $syncOutput "Syncing smoke-repo..." "sync command did not run for the fixture repo"

    $summaryJson = (& $binPath summary --format json | Out-String)
    Assert-Contains $summaryJson '"event_type": "commit"' "summary JSON did not contain a commit event"
    Assert-Contains $summaryJson '"message": "smoke commit"' "summary JSON did not include the fixture commit"

    $updateOutput = (& $binPath update | Out-String)
    Assert-Contains $updateOutput "Manual update required on Windows" "Windows update command did not report the manual reinstall path"

    $postUpdateStatus = (& $binPath status) | Out-String
    Assert-Contains $postUpdateStatus "devjournal daemon: not running" "binary was not runnable after Windows update smoke"

    Write-Host "Windows release smoke passed for $(Split-Path -Leaf $ArchivePath)"
}
finally {
    if ($binPath -and (Test-Path -LiteralPath $binPath)) {
        try {
            & $binPath stop | Out-Null
        }
        catch {
        }
    }

    if (Test-Path -LiteralPath $workRoot) {
        Remove-Item -LiteralPath $workRoot -Recurse -Force
    }
}
