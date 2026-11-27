<#
  Paceline installer for Windows 10 and 11.

    irm https://raw.githubusercontent.com/reginaldosilva27/paceline/main/install.ps1 | iex

  Options need the script-block form:

    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/reginaldosilva27/paceline/main/install.ps1))) -Uninstall

    -Uninstall        remove the app, its login entry and the Claude Code hook
    -Installer FILE   install from a local setup .exe instead of downloading
    -NoLaunch         do not start the app afterwards

  PACELINE_INSTALLER and PACELINE_NO_LAUNCH=1 in the environment do the same
  as -Installer and -NoLaunch, for the `irm | iex` form.

  The setup program comes from the latest GitHub release and installs for the
  current user only (no administrator rights). Files downloaded by PowerShell
  carry no "downloaded from the internet" mark, so SmartScreen does not block
  the unsigned installer.
#>
param(
    [switch]$Uninstall,
    [string]$Installer,
    [switch]$NoLaunch
)

# Everything runs inside a function: `exit` or `return` at the top level of a
# script piped into `iex` would close the user's PowerShell window.
function Invoke-PacelineInstall {
    param([bool]$Uninstall, [string]$Installer, [bool]$NoLaunch)

    $ErrorActionPreference = 'Stop'
    # Invoke-WebRequest is many times slower with the progress bar on in Windows PowerShell 5.1.
    $ProgressPreference = 'SilentlyContinue'

    $Repo = 'reginaldosilva27/paceline'
    $SetupUrl = "https://github.com/$Repo/releases/latest/download/Paceline-windows-x64-setup.exe"
    $UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Paceline'

    function Say([string]$Text) { Write-Host "==> $Text" -ForegroundColor Cyan }

    if ($env:OS -ne 'Windows_NT') {
        throw "This installer is for Windows. On macOS run: curl -fsSL https://raw.githubusercontent.com/$Repo/main/install.sh | bash"
    }

    function Find-Paceline {
        $default = Join-Path $env:LOCALAPPDATA 'Paceline\paceline.exe'
        if (Test-Path $default) { return $default }
        $entry = Get-ItemProperty $UninstallKey -ErrorAction SilentlyContinue
        if ($entry -and $entry.InstallLocation) {
            $exe = Join-Path $entry.InstallLocation.Trim('"') 'paceline.exe'
            if (Test-Path $exe) { return $exe }
        }
        return $null
    }

    function Stop-Paceline {
        $running = Get-Process -Name paceline -ErrorAction SilentlyContinue
        if ($running) {
            Say 'Closing the running Paceline'
            $running | Stop-Process -Force
            Start-Sleep -Milliseconds 800
        }
    }

    # Runs the app binary as a CLI and waits for it (it is a GUI program, so a
    # plain call would return before it finishes).
    function Invoke-PacelineCli([string]$Exe, [string[]]$Arguments) {
        $p = Start-Process -FilePath $Exe -ArgumentList $Arguments -Wait -PassThru -WindowStyle Hidden
        return $p.ExitCode
    }

    # ---------- uninstall ----------
    if ($Uninstall) {
        $exe = Find-Paceline
        if (-not $exe) { Say 'Paceline is not installed.'; return }
        Say 'Removing the Claude Code statusline hook (your previous statusline is restored)'
        [void](Invoke-PacelineCli $exe @('hook', 'uninstall'))
        Stop-Paceline
        Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Paceline' -ErrorAction SilentlyContinue
        $uninstaller = Join-Path (Split-Path $exe) 'uninstall.exe'
        if (Test-Path $uninstaller) {
            Say 'Running the uninstaller'
            Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait
            # The NSIS uninstaller hands off to a copy of itself; wait for the files to go.
            for ($i = 0; $i -lt 40 -and (Test-Path $exe); $i++) { Start-Sleep -Milliseconds 250 }
        }
        Say 'Done. Settings and limit history stay in ~\.paceline; delete that folder to remove them too.'
        return
    }

    # ---------- install ----------
    if (-not [Environment]::Is64BitOperatingSystem) {
        throw 'Paceline needs 64-bit Windows.'
    }

    $tempSetup = $null
    if ($Installer) {
        if (-not (Test-Path $Installer)) { throw "Installer not found: $Installer" }
        $setup = (Resolve-Path $Installer).Path
    } else {
        Say 'Downloading the latest Paceline release'
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        $tempSetup = Join-Path ([IO.Path]::GetTempPath()) ("Paceline-setup-{0}.exe" -f [guid]::NewGuid().ToString('N').Substring(0, 8))
        try {
            Invoke-WebRequest -Uri $SetupUrl -OutFile $tempSetup -UseBasicParsing
        } catch {
            throw "Download failed ($SetupUrl). If no release has been published yet, build from source: https://github.com/$Repo#build-from-source"
        }
        $setup = $tempSetup
    }

    try {
        Stop-Paceline
        Say 'Installing (current user, no administrator rights needed)'
        $p = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "The installer exited with code $($p.ExitCode)." }
    } finally {
        if ($tempSetup) { Remove-Item $tempSetup -Force -ErrorAction SilentlyContinue }
    }

    $exe = Find-Paceline
    if (-not $exe) { throw 'Installation finished, but paceline.exe was not found.' }

    if (-not $NoLaunch) {
        Say 'Starting Paceline'
        Start-Process -FilePath $exe
    }

    Write-Host ''
    Write-Host "Paceline is installed at $exe" -ForegroundColor Green
    Write-Host @"

Next:
  1. Click the Paceline icon in the notification area, next to the clock
     (it may be hidden under the ^ arrow), and choose "Enable live limits".
     Or from a terminal:  & "$exe" hook install
  2. Send any message in Claude Code. The percentages appear within seconds.
  3. Optional: Settings > Open at login. To keep the icon visible, drag it out
     of the ^ overflow onto the taskbar.

Uninstall:
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/$Repo/main/install.ps1))) -Uninstall
"@
}

if (-not $Installer) { $Installer = $env:PACELINE_INSTALLER }
Invoke-PacelineInstall -Uninstall:$Uninstall.IsPresent -Installer $Installer -NoLaunch:($NoLaunch.IsPresent -or $env:PACELINE_NO_LAUNCH -eq '1')
