#!/bin/env pwsh

$diagramsOut = "img/diagrams"
$drawIo = (Get-ChildItem -Recurse diagrams)

if (-not (Test-Path $diagramsOut)) {
    New-Item -ItemType Directory -Path $diagramsOut
}

foreach ($file in $drawIo) {
    Write-Host "Processing $file"
    $fName = $file.BaseName

    draw.io --crop -x -o $diagramsOut $file.FullName
    Write-Host "Generated $diagramsOut/$fName.pdf"
}