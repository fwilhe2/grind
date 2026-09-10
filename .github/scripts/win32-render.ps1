# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Run `grind-win32 <document> --render-to <bmp>` and wait for the frame, or fail.
#
# One function rather than four copies of it in `win32.yml`, because every part of it is a
# lesson paid for by a timed-out job — see that workflow's own comment for the three properties
# it has to have. In short: arguments quoted by .NET rather than joined by PowerShell, the wait
# is real because a GUI-subsystem process detaches at once, and the wait is bounded so that a
# shell which manages to put a dialog up on a session nobody is logged into fails in two minutes
# with its output shown rather than hanging until the runner kills the job.

function Invoke-Render {
    param(
        [Parameter(Mandatory = $true)][string] $Document,
        [Parameter(Mandatory = $true)][string] $Output,
        # W10's `--dark`: which palette the frame is drawn in. A flag rather than a registry
        # read, so that the output stays a function of the command line alone — the whole point
        # of this path is that two runs produce the same bytes.
        [switch] $Dark,
        [int] $TimeoutSeconds = 120
    )

    $exe = Join-Path (Get-Location) "target/release/grind-win32.exe"

    $info = [System.Diagnostics.ProcessStartInfo]::new($exe)
    # `ArgumentList` applies Windows' own quoting rules per element. `Arguments` — and
    # `Start-Process -ArgumentList` — would hand the whole thing over as one pre-joined string
    # and split a document whose name has a space in it back into two arguments.
    $info.ArgumentList.Add($Document)
    $info.ArgumentList.Add("--render-to")
    $info.ArgumentList.Add($Output)
    if ($Dark) { $info.ArgumentList.Add("--dark") }
    # Not inherited: .NET resolves a relative path against its own idea of the current
    # directory, which is not pwsh's.
    $info.WorkingDirectory = (Get-Location).Path
    $info.UseShellExecute = $false
    # A GUI-subsystem binary has no console, but it does inherit these — which is what makes
    # `main.rs`'s headless reporting readable here instead of trapped in a message box.
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true

    $process = [System.Diagnostics.Process]::Start($info)
    # Read before waiting: a pipe that fills up blocks the writer, and a blocked writer never
    # exits. Both are started as tasks so neither can be the one that fills.
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()

    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill($true)
        Write-Host $stdout.Result
        Write-Host $stderr.Result
        Write-Error "rendering $Output did not finish within $TimeoutSeconds seconds"
        exit 1
    }

    $out = $stdout.Result
    $err = $stderr.Result
    if ($out) { Write-Host $out }
    if ($err) { Write-Host $err }
    if ($process.ExitCode -ne 0) {
        Write-Error "rendering $Output failed with exit code $($process.ExitCode)"
        exit 1
    }
    if (-not (Test-Path $Output)) {
        Write-Error "rendering $Output reported success but wrote no file"
        exit 1
    }
    Write-Host "rendered $Output"
}
