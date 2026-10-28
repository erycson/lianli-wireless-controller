# Renders assets/icon.png (512 px) and assets/icon.ico (16-256 px) from vector shapes.
# Run with pwsh on Windows: pwsh assets/make_icon.ps1
Add-Type -AssemblyName System.Drawing
$dir = $PSScriptRoot

function Draw-Icon([int]$n) {
    $bmp = [System.Drawing.Bitmap]::new($n, $n)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Transparent)
    $s = $n / 100.0

    # dark rounded square
    $r = 22 * $s
    $path = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $path.AddArc(0, 0, 2 * $r, 2 * $r, 180, 90)
    $path.AddArc($n - 2 * $r - 1, 0, 2 * $r, 2 * $r, 270, 90)
    $path.AddArc($n - 2 * $r - 1, $n - 2 * $r - 1, 2 * $r, 2 * $r, 0, 90)
    $path.AddArc(0, $n - 2 * $r - 1, 2 * $r, 2 * $r, 90, 90)
    $path.CloseFigure()
    $g.FillPath([System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 14, 24, 48)), $path)

    $cyan = [System.Drawing.Color]::FromArgb(255, 0, 214, 230)
    $brush = [System.Drawing.SolidBrush]::new($cyan)

    # fan rotor: 5 curved blades around a hub, centered low-left
    $cx = 44 * $s; $cy = 56 * $s
    for ($i = 0; $i -lt 5; $i++) {
        $blade = [System.Drawing.Drawing2D.GraphicsPath]::new()
        $blade.AddBezier($cx, $cy, $cx + 4 * $s, $cy - 30 * $s, $cx + 26 * $s, $cy - 26 * $s, $cx + 30 * $s, $cy - 8 * $s)
        $blade.AddBezier($cx + 30 * $s, $cy - 8 * $s, $cx + 20 * $s, $cy - 14 * $s, $cx + 8 * $s, $cy - 10 * $s, $cx, $cy)
        $m = [System.Drawing.Drawing2D.Matrix]::new()
        $m.RotateAt(72 * $i, [System.Drawing.PointF]::new($cx, $cy))
        $blade.Transform($m)
        $g.FillPath($brush, $blade)
    }
    $hub = 9 * $s
    $g.FillEllipse([System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 14, 24, 48)), $cx - $hub, $cy - $hub, 2 * $hub, 2 * $hub)
    $g.FillEllipse($brush, $cx - $hub * 0.55, $cy - $hub * 0.55, 1.1 * $hub, 1.1 * $hub)

    # radio arcs, top-right
    $pen = [System.Drawing.Pen]::new($cyan, [Math]::Max(1.5, 6 * $s))
    $pen.StartCap = 'Round'; $pen.EndCap = 'Round'
    foreach ($k in 1..3) {
        $rr = (2 + 9 * $k) * $s
        $g.DrawArc($pen, 62 * $s - $rr, 36 * $s - $rr, 2 * $rr, 2 * $rr, -80, 70)
    }
    $g.Dispose()
    $bmp
}

$big = Draw-Icon 512
$big.Save("$dir\icon.png", [System.Drawing.Imaging.ImageFormat]::Png)

# ICO with embedded PNG frames (Vista+ format)
$sizes = 16, 24, 32, 48, 64, 128, 256
$frames = foreach ($sz in $sizes) {
    $ms = [System.IO.MemoryStream]::new()
    (Draw-Icon $sz).Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    , $ms.ToArray()
}
$out = [System.IO.BinaryWriter]::new([System.IO.File]::Create("$dir\icon.ico"))
$out.Write([uint16]0); $out.Write([uint16]1); $out.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $d = if ($sizes[$i] -ge 256) { 0 } else { $sizes[$i] }
    $out.Write([byte]$d); $out.Write([byte]$d); $out.Write([byte]0); $out.Write([byte]0)
    $out.Write([uint16]1); $out.Write([uint16]32)
    $out.Write([uint32]$frames[$i].Length); $out.Write([uint32]$offset)
    $offset += $frames[$i].Length
}
foreach ($f in $frames) { $out.Write($f) }
$out.Close()
