
Add-Type -AssemblyName System.Drawing
$source = "app-icon.png"
$dest = "app-icon-converted.png"
try {
    $image = [System.Drawing.Image]::FromFile($source)
    $image.Save($dest, [System.Drawing.Imaging.ImageFormat]::Png)
    $image.Dispose()
    Write-Host "Conversion successful"
} catch {
    Write-Error $_
    exit 1
}
