param(
    [Parameter(Mandatory = $true)]
    [string]$ArchivePath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Fail([string]$Message) {
    throw "FAIL: $Message"
}

function Write-Step([string]$Message) {
    Write-Host "[smoke] $Message"
}

function Assert-Contains([string]$Text, [string]$Needle, [string]$Label) {
    if (-not $Text.Contains($Needle)) {
        Write-Error $Text
        Fail $Label
    }
}

function Invoke-ExternalCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,

        [string[]]$ArgumentList = @(),

        [Parameter(Mandatory = $true)]
        [string]$Label,

        [int]$TimeoutSeconds = 30,

        [string]$WorkingDirectory = $null,

        [switch]$CaptureOutput = $true,

        [switch]$AllowFailure
    )

    Write-Step "$Label..."

    $process = New-Object System.Diagnostics.Process
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FilePath
    foreach ($arg in $ArgumentList) {
        [void]$startInfo.ArgumentList.Add($arg)
    }
    if ($WorkingDirectory) {
        $startInfo.WorkingDirectory = $WorkingDirectory
    }
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $CaptureOutput
    $startInfo.RedirectStandardError = $CaptureOutput
    $process.StartInfo = $startInfo

    try {
        [void]$process.Start()
        if ($CaptureOutput) {
            $stdoutTask = $process.StandardOutput.ReadToEndAsync()
            $stderrTask = $process.StandardError.ReadToEndAsync()
        }

        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            try {
                $process.Kill($true)
            }
            catch {
            }
            Fail "$Label timed out after $TimeoutSeconds seconds"
        }

        $process.WaitForExit()

        $stdout = ""
        $stderr = ""
        if ($CaptureOutput) {
            $stdout = $stdoutTask.GetAwaiter().GetResult()
            $stderr = $stderrTask.GetAwaiter().GetResult()
        }
        $combined = ($stdout, $stderr | Where-Object { $_ }) -join ""

        if (-not $AllowFailure -and $process.ExitCode -ne 0) {
            if ($combined) {
                Write-Host $combined
            }
            Fail "$Label failed with exit code $($process.ExitCode)"
        }

        if ($combined) {
            Write-Host $combined.TrimEnd()
        }

        return $combined
    }
    finally {
        $process.Dispose()
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
    Write-Step "Expanding release archive"
    Expand-Archive -LiteralPath $ArchivePath -DestinationPath $installDir -Force
    $binPath = Join-Path $installDir "devjournal.exe"
    if (-not (Test-Path -LiteralPath $binPath)) {
        Fail "Extracted archive does not contain devjournal.exe"
    }

    Write-Step "Creating fixture git repository"
    Push-Location $repoDir
    try {
        Invoke-ExternalCommand -FilePath "git" -ArgumentList @("init", "-b", "main") -Label "git init" -WorkingDirectory $repoDir | Out-Null
        Invoke-ExternalCommand -FilePath "git" -ArgumentList @("config", "user.name", "Smoke Tester") -Label "git config user.name" -WorkingDirectory $repoDir | Out-Null
        Invoke-ExternalCommand -FilePath "git" -ArgumentList @("config", "user.email", "smoke@example.com") -Label "git config user.email" -WorkingDirectory $repoDir | Out-Null
        Invoke-ExternalCommand -FilePath "git" -ArgumentList @("config", "commit.gpgsign", "false") -Label "git config commit.gpgsign" -WorkingDirectory $repoDir | Out-Null
        Set-Content -LiteralPath (Join-Path $repoDir "README.md") -Value "# smoke repo"
        Invoke-ExternalCommand -FilePath "git" -ArgumentList @("add", "README.md") -Label "git add README.md" -WorkingDirectory $repoDir | Out-Null
        Invoke-ExternalCommand -FilePath "git" -ArgumentList @("commit", "-m", "smoke commit") -Label "git commit fixture" -WorkingDirectory $repoDir | Out-Null
    }
    finally {
        Pop-Location
    }

    $configPath = (Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("config") -Label "devjournal config").Trim()
    $configDir = Split-Path -Parent $configPath
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null

    $repoTomlPath = $repoDir.Replace("'", "''")
    @"
[general]
poll_interval_secs = 1
author = "Smoke Tester"

[llm]
provider = "ollama"
model = "llama3.2"

[[repos]]
path = '$repoTomlPath'
name = "smoke-repo"
"@ | Set-Content -LiteralPath $configPath

    Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("start") -Label "devjournal start" -CaptureOutput:$false | Out-Null
    Start-Sleep -Seconds 2
    $runningStatus = Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("status") -Label "devjournal status (running)"
    Assert-Contains $runningStatus "devjournal daemon: running" "daemon did not report running after start"

    Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("stop") -Label "devjournal stop" | Out-Null
    $stoppedStatus = Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("status") -Label "devjournal status (stopped)"
    Assert-Contains $stoppedStatus "devjournal daemon: not running" "daemon did not report stopped after stop"

    $syncOutput = Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("sync") -Label "devjournal sync" -TimeoutSeconds 60
    Assert-Contains $syncOutput "Syncing smoke-repo..." "sync command did not run for the fixture repo"

    $summaryJson = Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("summary", "--format", "json") -Label "devjournal summary"
    Assert-Contains $summaryJson '"event_type": "commit"' "summary JSON did not contain a commit event"
    Assert-Contains $summaryJson '"message": "smoke commit"' "summary JSON did not include the fixture commit"

    $updateOutput = Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("update") -Label "devjournal update"
    Assert-Contains $updateOutput "Manual update required on Windows" "Windows update command did not report the manual reinstall path"

    $postUpdateStatus = Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("status") -Label "devjournal status (post-update)"
    Assert-Contains $postUpdateStatus "devjournal daemon: not running" "binary was not runnable after Windows update smoke"

    Write-Host "Windows release smoke passed for $(Split-Path -Leaf $ArchivePath)"
}
finally {
    if ($binPath -and (Test-Path -LiteralPath $binPath)) {
        try {
            Invoke-ExternalCommand -FilePath $binPath -ArgumentList @("stop") -Label "cleanup stop" -TimeoutSeconds 15 -AllowFailure | Out-Null
        }
        catch {
        }
    }

    if (Test-Path -LiteralPath $workRoot) {
        Write-Step "Cleaning up fixture workspace"
        Remove-Item -LiteralPath $workRoot -Recurse -Force
    }
}
