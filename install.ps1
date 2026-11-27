<#
  Limitlane installer for Windows 10 and 11.

    irm https://raw.githubusercontent.com/reginaldosilva27/limitlane/main/install.ps1 | iex

  Options need the script-block form:

    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/reginaldosilva27/limitlane/main/install.ps1))) -Uninstall

    -Uninstall        remove the app, its login entry and the Claude Code hook
    -Installer FILE   install from a local setup .exe instead of downloading
    -NoLaunch         do not start the app afterwards

  LIMITLANE_INSTALLER and LIMITLANE_NO_LAUNCH=1 in the environment do the same
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
function Invoke-LimitlaneInstall {
    param([bool]$Uninstall, [string]$Installer, [bool]$NoLaunch)

    $ErrorActionPreference = 'Stop'
    # Invoke-WebRequest is many times slower with the progress bar on in Windows PowerShell 5.1.
    $ProgressPreference = 'SilentlyContinue'

    $Repo = 'reginaldosilva27/limitlane'
    $SetupUrl = "https://github.com/$Repo/releases/latest/download/Limitlane-windows-x64-setup.exe"
    $UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Limitlane'

    function Say([string]$Text) { Write-Host "==> $Text" -ForegroundColor Cyan }

    if ($env:OS -ne 'Windows_NT') {
        throw "This installer is for Windows. On macOS run: curl -fsSL https://raw.githubusercontent.com/$Repo/main/install.sh | bash"
    }

    function Find-Limitlane {
        $default = Join-Path $env:LOCALAPPDATA 'Limitlane\limitlane.exe'
        if (Test-Path $default) { return $default }
        $entry = Get-ItemProperty $UninstallKey -ErrorAction SilentlyContinue
        if ($entry -and $entry.InstallLocation) {
            $exe = Join-Path $entry.InstallLocation.Trim('"') 'limitlane.exe'
            if (Test-Path $exe) { return $exe }
        }
        return $null
    }

    function Stop-Limitlane {
        $running = Get-Process -Name limitlane -ErrorAction SilentlyContinue
        if ($running) {
            Say 'Closing the running Limitlane'
            $running | Stop-Process -Force
            Start-Sleep -Milliseconds 800
        }
    }

    # Runs the command line and waits for it. The console CLI ships next to the
    # app; the app binary answers the same commands but is a GUI program, so a
    # plain call to it would return before it finishes.
    function Invoke-LimitlaneCli([string]$Exe, [string[]]$Arguments) {
        $cli = Join-Path (Split-Path $Exe) 'limitlane-cli.exe'
        if (Test-Path $cli) { $Exe = $cli }
        $p = Start-Process -FilePath $Exe -ArgumentList $Arguments -Wait -PassThru -NoNewWindow
        return $p.ExitCode
    }

    # ---------- uninstall ----------
    if ($Uninstall) {
        $exe = Find-Limitlane
        if (-not $exe) { Say 'Limitlane is not installed.'; return }
        Say 'Removing the Claude Code statusline hook (your previous statusline is restored)'
        [void](Invoke-LimitlaneCli $exe @('hook', 'uninstall'))
        Stop-Limitlane
        Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Limitlane' -ErrorAction SilentlyContinue
        $uninstaller = Join-Path (Split-Path $exe) 'uninstall.exe'
        if (Test-Path $uninstaller) {
            Say 'Running the uninstaller'
            Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait
            # The NSIS uninstaller hands off to a copy of itself; wait for the files to go.
            for ($i = 0; $i -lt 40 -and (Test-Path $exe); $i++) { Start-Sleep -Milliseconds 250 }
        }
        Say 'Done. Settings and limit history stay in ~\.limitlane; delete that folder to remove them too.'
        return
    }

    # ---------- install ----------
    if (-not [Environment]::Is64BitOperatingSystem) {
        throw 'Limitlane needs 64-bit Windows.'
    }

    $tempSetup = $null
    if ($Installer) {
        if (-not (Test-Path $Installer)) { throw "Installer not found: $Installer" }
        $setup = (Resolve-Path $Installer).Path
    } else {
        Say 'Downloading the latest Limitlane release'
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        $tempSetup = Join-Path ([IO.Path]::GetTempPath()) ("Limitlane-setup-{0}.exe" -f [guid]::NewGuid().ToString('N').Substring(0, 8))
        try {
            Invoke-WebRequest -Uri $SetupUrl -OutFile $tempSetup -UseBasicParsing
        } catch {
            throw "Download failed ($SetupUrl). If no release has been published yet, build from source: https://github.com/$Repo#build-from-source"
        }
        $setup = $tempSetup
    }

    try {
        Stop-Limitlane
        Say 'Installing (current user, no administrator rights needed)'
        $p = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "The installer exited with code $($p.ExitCode)." }
    } finally {
        if ($tempSetup) { Remove-Item $tempSetup -Force -ErrorAction SilentlyContinue }
    }

    $exe = Find-Limitlane
    if (-not $exe) { throw 'Installation finished, but limitlane.exe was not found.' }

    if (-not $NoLaunch) {
        Say 'Starting Limitlane'
        Start-Process -FilePath $exe
    }

    Write-Host ''
    Write-Host "Limitlane is installed at $exe" -ForegroundColor Green
    Write-Host @"

Next:
  1. Click the Limitlane icon in the notification area, next to the clock
     (it may be hidden under the ^ arrow), and choose "Enable live limits".
     Or from a terminal:  ~/AppData/Local/Limitlane/limitlane-cli.exe hook install
  2. Send any message in Claude Code. The percentages appear within seconds.
  3. Optional: Settings > Open at login. To keep the icon visible, drag it out
     of the ^ overflow onto the taskbar.

Uninstall:
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/$Repo/main/install.ps1))) -Uninstall
"@
}

if (-not $Installer) { $Installer = $env:LIMITLANE_INSTALLER }
Invoke-LimitlaneInstall -Uninstall:$Uninstall.IsPresent -Installer $Installer -NoLaunch:($NoLaunch.IsPresent -or $env:LIMITLANE_NO_LAUNCH -eq '1')
