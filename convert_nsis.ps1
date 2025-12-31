
Add-Type -AssemblyName System.Drawing

$sourcePath = "$PSScriptRoot\src-tauri\icons\128x128.png"
$headerDest = "$PSScriptRoot\src-tauri\icons\header.bmp"
$sidebarDest = "$PSScriptRoot\src-tauri\icons\sidebar.bmp"

if (Test-Path $sourcePath) {
    $img = [System.Drawing.Image]::FromFile($sourcePath)
    $bmp = new-object System.Drawing.Bitmap $img
    
    # Save as BMP
    $bmp.Save($headerDest, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $bmp.Save($sidebarDest, [System.Drawing.Imaging.ImageFormat]::Bmp)
    
    $img.Dispose()
    $bmp.Dispose()
    
    Write-Host "Converted 128x128.png to header.bmp and sidebar.bmp for NSIS."
}
else {
    Write-Error "Source image not found: $sourcePath"
}
