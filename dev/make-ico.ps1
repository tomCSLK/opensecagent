# Writes packaging\windows\opensecagent.ico from packaging\linux\opensecagent.svg.
#
# The icon of opensecagent.exe (build.rs embeds it on Windows) and of the installer. Microsoft Edge renders the SVG
# once at 256 px with a transparent background, System.Drawing scales it down to the smaller sizes, and the ICO file
# holds every size as PNG (allowed since Windows Vista), so no image library is needed. Run it again after the SVG
# has changed and commit the result. Windows only.
#
# Usage, from any folder:
#     powershell -ExecutionPolicy Bypass -File dev\make-ico.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$source = Join-Path $root 'packaging\linux\opensecagent.svg'
$target = Join-Path $root 'packaging\windows\opensecagent.ico'
# the sizes Windows asks for most (Explorer, taskbar, title bar, Alt+Tab, at 100 to 200 % scaling)
$sizes = @(16, 24, 32, 48, 64, 256)

$edge = @(
    (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'),
    (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $edge) { throw 'Microsoft Edge is missing' }

Add-Type -AssemblyName System.Drawing

$work = Join-Path ([IO.Path]::GetTempPath()) ("opensecagent-ico-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $work | Out-Null
try {
    $svg = Get-Content -Raw -Encoding UTF8 $source
    $html = '<!doctype html><html><head><style>html,body{margin:0;background:transparent}' +
        'svg{display:block;width:256px;height:256px}</style></head><body>' + $svg + '</body></html>'
    [IO.File]::WriteAllText((Join-Path $work 'icon.html'), $html)
    $big = Join-Path $work 'icon256.png'
    $url = 'file:///' + ((Join-Path $work 'icon.html') -replace '\\', '/')
    $edgeArgs = @('--headless=new', '--disable-gpu', '--hide-scrollbars', '--default-background-color=00000000',
        "--user-data-dir=$work\profile", '--window-size=256,256', "--screenshot=$big", $url)
    Start-Process -FilePath $edge -ArgumentList $edgeArgs -Wait -WindowStyle Hidden
    if (-not (Test-Path $big)) { throw 'Edge wrote no screenshot' }

    $original = [Drawing.Image]::FromFile($big)
    try {
        if ($original.Width -ne 256 -or $original.Height -ne 256) {
            throw "Edge rendered $($original.Width)x$($original.Height) instead of 256x256"
        }
        # a corner outside the rounded square must be transparent, or the background was not
        if ($original.GetPixel(0, 0).A -ne 0) { throw 'the background of the rendered icon is not transparent' }
        $images = foreach ($size in $sizes) {
            if ($size -eq 256) {
                , [IO.File]::ReadAllBytes($big)
                continue
            }
            $bitmap = New-Object Drawing.Bitmap $size, $size, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
            $graphics = [Drawing.Graphics]::FromImage($bitmap)
            $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $graphics.CompositingQuality = [Drawing.Drawing2D.CompositingQuality]::HighQuality
            $graphics.DrawImage($original, 0, 0, $size, $size)
            $graphics.Dispose()
            $stream = New-Object IO.MemoryStream
            $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
            $bitmap.Dispose()
            , $stream.ToArray()
        }
    } finally {
        $original.Dispose()
    }

    # ICONDIR, then one ICONDIRENTRY per image, then the images
    $out = New-Object IO.MemoryStream
    $writer = New-Object IO.BinaryWriter $out
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $side = if ($sizes[$i] -ge 256) { 0 } else { $sizes[$i] }  # 0 stands for 256
        $writer.Write([byte]$side); $writer.Write([byte]$side); $writer.Write([byte]0); $writer.Write([byte]0)
        $writer.Write([uint16]1); $writer.Write([uint16]32)
        $writer.Write([uint32]$images[$i].Length); $writer.Write([uint32]$offset)
        $offset += $images[$i].Length
    }
    foreach ($image in $images) { $writer.Write([byte[]]$image) }
    $writer.Flush()
    New-Item -ItemType Directory -Force (Split-Path -Parent $target) | Out-Null
    [IO.File]::WriteAllBytes($target, $out.ToArray())
    Write-Host "packaging\windows\opensecagent.ico: $($sizes.Count) images, $($out.Length) bytes"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
