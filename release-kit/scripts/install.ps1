<#
.SYNOPSIS
Install an app for the current user on Windows, without administrator rights (release-kit of appkit).

.DESCRIPTION
Downloads the Windows build of the app from its GitHub release, checks it against the SHA256SUMS of
that release, and installs it in the way the app is built:

  hub    a program with its own `install` command (a .zip with the program, or the bare program):
         runs `<program> install`, which copies it to its fixed place
  nsis   an NSIS setup: runs it silently (/S)
  inno   an Inno Setup: runs it silently (/VERYSILENT /SUPPRESSMSGBOXES /CURRENTUSER /NORESTART)

A file this script downloads carries no Mark-of-the-Web, so SmartScreen does not stop the setup.
What is checked instead is that the file is the one the release lists in SHA256SUMS: nothing runs
when it does not match. (That proves the file is whole and the one of the release, not who made the
release.)

The copy attached to a release is stamped by the release workflow: it knows its repository, its app
and its channel (the release it came with, or the rolling build `latest`) and runs without
parameters, also through Invoke-Expression:

  powershell -ExecutionPolicy Bypass -c "irm https://github.com/OWNER/NAME/releases/download/latest/install.ps1 | iex"

Run it in your own PowerShell, not elevated: the apps install for one user, and an elevated session
belongs to the administrator account, whose profile would get the app.

Parameters (all optional):

  -Version TAG        a release to install instead of the channel of this copy
  -Rolling            the rolling build of main (the pre-release `latest`) instead
  -File PATH          a build you already have: nothing is downloaded; the checksum then comes from
                      -Sums, -Sha256 or a SHA256SUMS file next to it
  -Sums PATH          a SHA256SUMS file ("<hash>  <file>" per line)
  -Sha256 HEX         the expected SHA-256 of the file
  -InstallArgs A,B    more arguments for the install step (`<program> install ...` or the setup)
  -AllowElevated      install although the session is elevated (a build machine)
  -Repo, -App, -Kind, -Program, -ReleaseAsset, -RollingAsset
                      what the release workflow stamps into this copy; given here, they win (for a
                      copy that is not stamped). -ReleaseAsset may hold {version} (the tag without
                      its v) and {triple} (x86_64-pc-windows-msvc).
  -ApiUrl URL         the GitHub API (default https://api.github.com); only https, or a local address
                      (the tests of release-kit run it against a server of their own). A token in
                      GH_TOKEN is sent to this address and nowhere else.

.EXAMPLE
.\install.ps1 -InstallArgs '--autostart'
#>

# Everything runs in a script block of its own, which takes the parameters: run through
# Invoke-Expression, the script sets no variable, strict mode or preference of the session of the
# person who runs it. `@args` hands named parameters and switches on as they were given.
& {
    param(
        [string] $Version = '',
        [switch] $Rolling,
        [string] $File = '',
        [string] $Sums = '',
        [string] $Sha256 = '',
        [string[]] $InstallArgs = @(),
        [switch] $AllowElevated,
        [string] $Repo = '',
        [string] $App = '',
        [string] $Kind = '',
        [string] $Program = '',
        [string] $ReleaseAsset = '',
        [string] $RollingAsset = '',
        [string] $ApiUrl = 'https://api.github.com'
    )
    Set-StrictMode -Version Latest
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'

    # release-kit:stamp:begin (filled by the publish action of release-kit; empty in the source)
    $StampRepo = ''
    $StampChannel = ''
    $StampApp = ''
    $StampKind = ''
    $StampProgram = ''
    $StampReleaseAsset = ''
    $StampRollingAsset = ''
    # release-kit:stamp:end

    # A plain word: it goes into a URL, a file name and a command line. `\z`, not `$`: `$` also
    # matches before a final line break.
    $plain = '^[A-Za-z0-9._-]+\z'

    function Exit-Install([string] $Message) {
        throw "install.ps1: $Message"
    }

    function Select-Given([string] $Given, [string] $Stamped) {
        if ($Given -ne '') { return $Given }
        return $Stamped
    }

    # A private repository answers an anonymous download with 404. With a token in GH_TOKEN (or
    # GITHUB_TOKEN; read access to the repository is enough) a file comes through the API instead:
    # the asset is looked up by name and fetched from its API address, and the token is not sent on
    # to the storage host the API redirects to.
    $token = ''
    if ($env:GH_TOKEN) { $token = $env:GH_TOKEN } elseif ($env:GITHUB_TOKEN) { $token = $env:GITHUB_TOKEN }
    # The token goes to this address: https, or a loopback address (tests), never plain http to another host.
    $ApiUrl = $ApiUrl.TrimEnd('/')
    if ($ApiUrl -cnotmatch '^(https://|http://(127\.0\.0\.1|localhost)(:[0-9]+)?(/|\z))') {
        Exit-Install "-ApiUrl must be an https address (or a local one): $ApiUrl"
    }
    $apiHeaders = @{ 'User-Agent' = 'release-kit-install' }
    if ($token -ne '') { $apiHeaders['Authorization'] = "Bearer $token" }

    # One request to the address of an asset at the API, without following a redirect: the API answers 302 to
    # the storage host, and the token must not go along. No cmdlet does this the same way in both PowerShells
    # (`-MaximumRedirection 0` throws in Windows PowerShell 5.1 without handing over the response), so
    # HttpWebRequest, which both have. Returns the address to go to, or '' when the file itself came in this
    # answer (written to $OutFile).
    function Get-AssetRedirect([string] $Url, [hashtable] $Headers, [string] $OutFile) {
        $request = [System.Net.HttpWebRequest]::Create($Url)
        $request.AllowAutoRedirect = $false
        $request.Method = 'GET'
        $request.UserAgent = $Headers['User-Agent']
        $request.Accept = $Headers['Accept']
        $request.Headers['Authorization'] = $Headers['Authorization']
        try { $response = $request.GetResponse() }
        catch [System.Net.WebException] {
            $response = $_.Exception.Response
            if (-not $response) { throw }
        }
        try {
            $code = [int] $response.StatusCode
            if ($code -ge 300 -and $code -lt 400) {
                $location = [string] $response.Headers['Location']
                if ($location -eq '') { throw "the answer of $Url is a redirect without a Location" }
                # a relative Location is relative to the address that answered
                return (New-Object System.Uri (New-Object System.Uri $Url), $location).AbsoluteUri
            }
            if ($code -lt 200 -or $code -ge 300) { throw "the answer of $Url is $code" }
            $stream = $response.GetResponseStream()
            try {
                $file = [System.IO.File]::Create($OutFile)
                try { $stream.CopyTo($file) } finally { $file.Dispose() }
            }
            finally { $stream.Dispose() }
            return ''
        }
        finally { $response.Dispose() }
    }

    function Get-ReleaseFile([string] $From, [string] $Channel, [string] $Name, [string] $OutFile) {
        if ($token -eq '') {
            Invoke-WebRequest -Uri "https://github.com/$From/releases/download/$Channel/$Name" -OutFile $OutFile -UseBasicParsing
            return
        }
        $release = Invoke-RestMethod -Uri "$ApiUrl/repos/$From/releases/tags/$Channel" -Headers $apiHeaders -UseBasicParsing
        $asset = @($release.assets | Where-Object { $_.name -ceq $Name }) | Select-Object -First 1
        if (-not $asset) { throw "the release $Channel of $From has no file $Name" }
        $headers = @{} + $apiHeaders
        $headers['Accept'] = 'application/octet-stream'
        $location = Get-AssetRedirect -Url $asset.url -Headers $headers -OutFile $OutFile
        if ($location -eq '') { return }
        Invoke-WebRequest -Uri $location -OutFile $OutFile -UseBasicParsing
    }

    $repo = Select-Given $Repo $StampRepo
    $app = Select-Given $App $StampApp
    $kind = Select-Given $Kind $StampKind
    $program = Select-Given $Program $StampProgram
    $releaseAsset = Select-Given $ReleaseAsset $StampReleaseAsset
    $rollingAsset = Select-Given $RollingAsset $StampRollingAsset

    if ($app -eq '') { $app = 'app' }
    if ($app -cnotmatch $plain) { Exit-Install "-App must be a plain word: $app" }
    if (@('hub', 'nsis', 'inno') -cnotcontains $kind) {
        Exit-Install "-Kind must be hub, nsis or inno (this copy is not stamped: give -Kind and the rest)"
    }
    if ($kind -eq 'hub' -and $program -cnotmatch $plain) {
        Exit-Install '-Program must be the file name of the program'
    }

    # The app belongs to one user: elevated, it would be the administrator account.
    $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $AllowElevated -and $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Exit-Install 'do not run this elevated: it installs for one user, without administrator rights (-AllowElevated for a build machine)'
    }

    $work = Join-Path ([IO.Path]::GetTempPath()) ("$app-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        if ($File -eq '') {
            if ($repo -cnotmatch '^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+\z') {
                Exit-Install '-Repo must be OWNER/NAME (this copy is not stamped)'
            }
            if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
                Exit-Install "no build for Windows on $env:PROCESSOR_ARCHITECTURE"
            }
            # Windows PowerShell 5.1 may offer a fixed set without TLS 1.2: it is added, nothing taken
            # away. SystemDefault (0, what pwsh has) lets the system choose, TLS 1.3 included: left alone.
            $offered = [Net.ServicePointManager]::SecurityProtocol
            if ([int] $offered -ne 0 -and -not ($offered -band [Net.SecurityProtocolType]::Tls12)) {
                [Net.ServicePointManager]::SecurityProtocol = $offered -bor [Net.SecurityProtocolType]::Tls12
            }

            # The channel: what was asked, else what this copy was stamped with, else the newest
            # published release (and the rolling build when there is none yet).
            $channel = $StampChannel
            if ($Version -ne '') { $channel = $Version }
            elseif ($Rolling) { $channel = 'latest' }
            if ($channel -eq '') {
                Write-Output "looking for the newest release of $repo"
                try {
                    $newest = Invoke-RestMethod -Uri "$ApiUrl/repos/$repo/releases/latest" -Headers $apiHeaders -UseBasicParsing
                    $channel = [string] $newest.tag_name
                }
                catch {
                    # Only "there is none" (404) means the rolling build; no network or a rate limit
                    # stops instead of quietly installing something else.
                    $response = $_.Exception.PSObject.Properties['Response']
                    if ($response -and $response.Value -and [int] $response.Value.StatusCode -eq 404) {
                        Write-Output 'no published release yet: taking the rolling build of main'
                        $channel = 'latest'
                    }
                    else { throw }
                }
            }
            if ($channel -cnotmatch $plain) {
                Exit-Install "the version has a character that is not allowed: $channel"
            }
            if ($channel -eq 'latest') {
                $name = $rollingAsset.Replace('{triple}', 'x86_64-pc-windows-msvc')
            }
            else {
                $name = $releaseAsset.Replace('{version}', ($channel -replace '^v', '')).Replace('{triple}', 'x86_64-pc-windows-msvc')
            }
            if ($name -eq '') { Exit-Install "the channel $channel has no build for Windows" }
            if ($name -cnotmatch $plain) { Exit-Install "the file name is not a plain word: $name" }

            Write-Output "downloading $name ($channel)"
            $path = Join-Path $work $name
            Get-ReleaseFile -From $repo -Channel $channel -Name $name -OutFile $path
            try {
                Get-ReleaseFile -From $repo -Channel $channel -Name 'SHA256SUMS' -OutFile (Join-Path $work 'SHA256SUMS')
            }
            catch {
                if ($channel -eq 'latest') {
                    # It cannot be told from here whether a build is being published or the last one stopped half way.
                    Exit-Install 'SHA256SUMS of the rolling build is missing: either a build is being published right now (try again in a few minutes), or its last publish failed and the next green build of the app brings them back'
                }
                throw
            }
            $Sums = Join-Path $work 'SHA256SUMS'
        }
        else {
            if (-not (Test-Path -LiteralPath $File -PathType Leaf)) { Exit-Install "no such file: $File" }
            # The full path: a bare name would be checked here but run from the PATH (an old install).
            $path = (Resolve-Path -LiteralPath $File).ProviderPath
        }

        # The checksum: never skipped. A file you have already needs one of the three sources.
        $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        $expected = $Sha256
        if ($expected -eq '') {
            if ($Sums -eq '') {
                $beside = Join-Path (Split-Path -Parent $path) 'SHA256SUMS'
                if (Test-Path -LiteralPath $beside -PathType Leaf) { $Sums = $beside }
            }
            if ($Sums -eq '') { Exit-Install "no checksum for ${path}: give -Sums FILE or -Sha256 HEX" }
            if (-not (Test-Path -LiteralPath $Sums -PathType Leaf)) { Exit-Install "no such file: $Sums" }
            $leaf = Split-Path -Leaf $path
            foreach ($line in Get-Content -LiteralPath $Sums) {
                # Lines are "<hash>  <file>" (or "<hash> *<file>"); the file name must match whole.
                if ($line -match '^([0-9A-Fa-f]{64})\s+\*?(.+)\z' -and $Matches[2] -ceq $leaf) {
                    $expected = $Matches[1]
                    break
                }
            }
            if ($expected -eq '') { Exit-Install "$leaf is not listed in $Sums" }
        }
        if ($actual -ne $expected.ToLowerInvariant()) {
            Exit-Install "the checksum of $path does not match (expected $($expected.ToLowerInvariant()), got $actual): nothing was installed"
        }
        Write-Output 'checksum ok'

        if ($kind -eq 'hub') {
            $exe = $path
            if ($path -like '*.zip') {
                # Every entry must land inside the folder it is unpacked into; checked before anything
                # is written. ExtractToDirectory refuses such a path as well, and throws on any error
                # (Expand-Archive only reports one and goes on).
                $unpacked = Join-Path $work 'unpacked'
                $root = [IO.Path]::GetFullPath($unpacked) + [IO.Path]::DirectorySeparatorChar
                Add-Type -AssemblyName System.IO.Compression.FileSystem
                $zip = [IO.Compression.ZipFile]::OpenRead($path)
                try {
                    foreach ($entry in $zip.Entries) {
                        $target = [IO.Path]::GetFullPath([IO.Path]::Combine($root, $entry.FullName))
                        if (-not $target.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
                            Exit-Install "the archive holds a path that leaves its folder ($($entry.FullName)): nothing was installed"
                        }
                    }
                }
                finally {
                    $zip.Dispose()
                }
                [IO.Compression.ZipFile]::ExtractToDirectory($path, $unpacked)
                $exe = Join-Path $unpacked $program
                if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { Exit-Install "the archive holds no $program" }
            }
            $newVersion = (& $exe --version)
            if ($LASTEXITCODE -ne 0) { Exit-Install 'the program does not run' }
            Write-Output "installing $newVersion"
            # `install` copies the program to its fixed place; its exit status is ours.
            & $exe install @InstallArgs
            if ($LASTEXITCODE -ne 0) { Exit-Install "$program install failed (see above)" }
            Write-Output "installed $newVersion"
        }
        else {
            if ($kind -eq 'nsis') {
                $flags = @('/S')
            }
            else {
                $flags = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/CURRENTUSER', '/NORESTART')
            }
            # Start-Process joins the arguments with spaces: one with a space in it is quoted here.
            foreach ($arg in $InstallArgs) {
                if ($arg -match '\s') { $flags += '"' + $arg + '"' } else { $flags += $arg }
            }
            Write-Output "running the setup silently ($($flags -join ' '))"
            $proc = Start-Process -FilePath $path -ArgumentList $flags -Wait -PassThru
            if ($proc.ExitCode -ne 0) { Exit-Install "the setup failed with exit code $($proc.ExitCode)" }
            Write-Output "installed $app"
        }
    }
    finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
} @args
