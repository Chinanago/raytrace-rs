# raytrace-rs installer for Windows (Windows PowerShell 5.1 and PowerShell 7; no admin rights).
#
#   irm https://raw.githubusercontent.com/Chinanago/raytrace-rs/main/install.ps1 | iex
#
# Options (when the file is saved and run as a script):
#   -Version v0.12.0   install this release instead of the latest one
#   -From C:\path\raytrace-rs-windows-x64.zip   install from a local zip (no download)
#   -Uninstall         remove the install folder and its PATH entry
# Through `irm | iex` set $env:RAYTRACE_VERSION first to pick a release.
#
# This file is ASCII only on purpose: Windows PowerShell 5.1 reads a file without a byte order
# mark in the ANSI code page, and the messages must read correctly in any console.
# The Japanese guide is docs/AI.md.
param(
    [string]$Version = $env:RAYTRACE_VERSION,
    [string]$From = $env:RAYTRACE_INSTALL_FROM,
    [switch]$Uninstall,
    [string]$BaseUrl = $env:RAYTRACE_RELEASES_URL
)

function Install-RaytraceRs {
    param(
        [string]$Version,
        [string]$From,
        [bool]$DoUninstall,
        [string]$BaseUrl
    )
    # Preference variables set here end with this function, so a caller using `| iex` keeps its own.
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    try {
        # Windows PowerShell 5.1 may default to TLS 1.0, which GitHub refuses.
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    } catch {
        # Not available on this platform; the default is fine.
    }

    if (-not $BaseUrl) { $BaseUrl = 'https://github.com/Chinanago/raytrace-rs/releases' }
    $BaseUrl = $BaseUrl.TrimEnd('/')
    if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set.' }
    $installDir = Join-Path $env:LOCALAPPDATA 'Programs\raytrace-rs'

    function Test-SamePath([string]$A, [string]$B) {
        return ($A.TrimEnd('\') -ieq $B.TrimEnd('\'))
    }

    function Get-PathEntries([string]$Value) {
        if (-not $Value) { return @() }
        return @($Value -split ';' | Where-Object { $_ })
    }

    function Add-ToUserPath([string]$Dir) {
        $entries = @(Get-PathEntries ([Environment]::GetEnvironmentVariable('Path', 'User')))
        $found = $false
        foreach ($entry in $entries) {
            if (Test-SamePath $entry $Dir) { $found = $true }
        }
        if (-not $found) {
            $entries = @($entries) + $Dir
            [Environment]::SetEnvironmentVariable('Path', ($entries -join ';'), 'User')
            Write-Host "Added $Dir to your user PATH."
        }
        # Also make it work in this window.
        $inSession = $false
        foreach ($entry in (Get-PathEntries $env:Path)) {
            if (Test-SamePath $entry $Dir) { $inSession = $true }
        }
        if (-not $inSession) { $env:Path = $env:Path + ';' + $Dir }
    }

    function Remove-FromUserPath([string]$Dir) {
        $entries = @(Get-PathEntries ([Environment]::GetEnvironmentVariable('Path', 'User')))
        $kept = @($entries | Where-Object { -not (Test-SamePath $_ $Dir) })
        if ($kept.Count -ne $entries.Count) {
            [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
            Write-Host "Removed $Dir from your user PATH."
        }
    }

    function Remove-Folder([string]$Dir) {
        if (-not (Test-Path -LiteralPath $Dir)) { return }
        try {
            Remove-Item -LiteralPath $Dir -Recurse -Force
        } catch {
            throw ('Could not remove ' + $Dir + ' (' + $_.Exception.Message + '). ' +
                'Quit programs that use raytrace-rs (Claude Desktop, Claude Code, Codex, raytrace-gui) and run this script again.')
        }
    }

    function Confirm-Checksum([string]$File, [string]$SumFile) {
        $text = Get-Content -LiteralPath $SumFile -Raw
        if (-not $text -or -not $text.Trim()) { throw "The checksum file is empty: $SumFile" }
        $expected = ($text.Trim() -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            throw ('Checksum mismatch for ' + (Split-Path -Leaf $File) + ': expected ' + $expected + ', got ' + $actual + '. Nothing was installed.')
        }
        Write-Host "Checksum verified (sha256 $actual)"
    }

    function Write-Setup([string]$Exe, [string]$Share) {
        $json = $Exe.Replace('\', '\\')
        Write-Host ''
        Write-Host 'Set up your AI client (copy and paste):'
        Write-Host ''
        Write-Host 'Claude Code'
        Write-Host ('  claude mcp add raytrace-rs -- "' + $Exe + '" mcp')
        Write-Host '  (add --scope user after "add" to use it in every project)'
        Write-Host '  or, with the plugin (skill and MCP server; needs raytrace-rs on PATH):'
        Write-Host '  /plugin install raytrace-rs --marketplace Chinanago/raytrace-rs'
        Write-Host '  Check with /mcp inside Claude Code.'
        Write-Host ''
        Write-Host 'Claude Desktop'
        Write-Host '  Recommended: open raytrace-rs-windows.mcpb from'
        Write-Host ('    ' + $BaseUrl + '/latest/download/raytrace-rs-windows.mcpb')
        Write-Host '  Or add this to %APPDATA%\Claude\claude_desktop_config.json'
        Write-Host '  (Settings > Developer > Edit Config opens the file; Claude Desktop does not read'
        Write-Host '  your PATH, so the full path is needed), then quit and restart Claude Desktop:'
        Write-Host ''
        Write-Host '  {'
        Write-Host '    "mcpServers": {'
        Write-Host '      "raytrace-rs": {'
        Write-Host ('        "command": "' + $json + '",')
        Write-Host '        "args": ["mcp"]'
        Write-Host '      }'
        Write-Host '    }'
        Write-Host '  }'
        Write-Host ''
        Write-Host 'Codex: add this to ~/.codex/config.toml (%USERPROFILE%\.codex\config.toml)'
        Write-Host ''
        Write-Host '  [mcp_servers.raytrace-rs]'
        Write-Host ("  command = '" + $Exe + "'")
        Write-Host '  args = ["mcp"]'
        Write-Host ''
        Write-Host ('Examples, schemas, docs and the GUI (raytrace-gui.exe): ' + $Share)
        Write-Host ('Guide (Japanese): ' + (Join-Path $Share 'docs\AI.md'))
    }

    if ($DoUninstall) {
        Remove-Folder $installDir
        Remove-FromUserPath $installDir
        Write-Host "Removed $installDir."
        Write-Host ''
        Write-Host 'If you registered the MCP server, remove it too:'
        Write-Host '  Claude Code:    claude mcp remove raytrace-rs'
        Write-Host '  Claude Desktop: delete the raytrace-rs entry from claude_desktop_config.json'
        Write-Host '  Codex:          delete the [mcp_servers.raytrace-rs] section from config.toml'
        Write-Host 'Settings and caches in %LOCALAPPDATA%\Raytrace are kept.'
        return
    }

    if (-not [Environment]::Is64BitOperatingSystem) { throw 'raytrace-rs needs 64-bit Windows.' }

    $work = Join-Path ([IO.Path]::GetTempPath()) ('raytrace-install-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work | Out-Null
    $stage = $null
    try {
        if ($From) {
            if (-not (Test-Path -LiteralPath $From -PathType Leaf)) { throw "No such file: $From" }
            $zip = (Resolve-Path -LiteralPath $From).Path
            $sumFile = $zip + '.sha256'
            if (Test-Path -LiteralPath $sumFile) { Confirm-Checksum $zip $sumFile }
        } else {
            if ($Version) {
                $tag = 'v' + $Version.TrimStart('v')
                $asset = 'raytrace-rs-' + $tag.Substring(1) + '-windows-x64.zip'
                $url = $BaseUrl + '/download/' + $tag + '/' + $asset
            } else {
                $asset = 'raytrace-rs-windows-x64.zip'
                $url = $BaseUrl + '/latest/download/' + $asset
            }
            $zip = Join-Path $work $asset
            Write-Host "Downloading $url"
            try {
                Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
            } catch {
                throw ('Download failed: ' + $url + ' (' + $_.Exception.Message + ')')
            }
            $sumFile = $zip + '.sha256'
            $haveSum = $true
            try {
                Invoke-WebRequest -Uri ($url + '.sha256') -OutFile $sumFile -UseBasicParsing
            } catch {
                $haveSum = $false
            }
            if ($haveSum) {
                Confirm-Checksum $zip $sumFile
            } else {
                Write-Warning "$asset.sha256 could not be downloaded; the file was not verified."
            }
        }

        $parent = Split-Path -Parent $installDir
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
        # Extract next to the target so that the final move stays on one volume.
        $stage = Join-Path $parent ('.raytrace-rs-install-' + [Guid]::NewGuid().ToString('N'))
        Expand-Archive -LiteralPath $zip -DestinationPath $stage -Force
        $top = Get-ChildItem -LiteralPath $stage -Directory |
            Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'raytrace-rs.exe') } |
            Select-Object -First 1
        if (-not $top) { throw 'The zip does not contain raytrace-rs.exe.' }
        try {
            # Files from a browser download carry a "downloaded from the internet" mark.
            Get-ChildItem -LiteralPath $top.FullName -Recurse -File | Unblock-File
        } catch {
            Write-Warning 'Could not clear the downloaded-file mark (harmless).'
        }

        Remove-Folder $installDir
        Move-Item -LiteralPath $top.FullName -Destination $installDir
    } finally {
        if ($stage -and (Test-Path -LiteralPath $stage)) { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
    Write-Host "Installed raytrace-rs in $installDir"

    $exe = Join-Path $installDir 'raytrace-rs.exe'
    $versionText = ''
    try {
        $versionText = (& $exe --version | Out-String).Trim()
    } catch {
        throw ('The installed raytrace-rs.exe does not start: ' + $_.Exception.Message)
    }
    if ($LASTEXITCODE -ne 0) { throw 'The installed raytrace-rs.exe does not start.' }
    Write-Host ''
    Write-Host ($versionText + ' is ready.')

    Add-ToUserPath $installDir
    Write-Host 'Open a new terminal window to use raytrace-rs by name. AI clients below use the full path.'
    Write-Setup $exe $installDir
}

Install-RaytraceRs -Version $Version -From $From -DoUninstall ([bool]$Uninstall) -BaseUrl $BaseUrl
