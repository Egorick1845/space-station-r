# Синхронизация спрайтов из сборки «Мини-станции» в assets/sprites/ss14.
# Источник: локальный чекаут C:\ss14\mini-station-goob (зеркало
# github.com/ministation/mini-station-goob). Добавляйте пути в $Sprites
# — относительные к Resources/Textures, с сохранением структуры.

$Source = "C:\ss14\mini-station-goob\Resources\Textures"
$Dest   = Join-Path $PSScriptRoot "..\assets\sprites\ss14"

# Относительные пути: файлы *.png или папки *.rsi целиком
$Sprites = @(
    "Mobs/Animals/monkey.rsi",
    "Mobs/Ghosts/ghost_human.rsi",
    "Objects/Tools/crowbar.rsi",
    "Structures/Walls/solid.rsi",
    "Structures/Doors/Airlocks/Standard/basic.rsi",
    "Tiles/steel.png",
    "Tiles/dark.png",
    "Tiles/blue.png"
)

foreach ($rel in $Sprites) {
    $src = Join-Path $Source $rel
    $dst = Join-Path $Dest ($rel -replace '/', '\')
    if (-not (Test-Path $src)) {
        Write-Warning "нет в источнике: $rel"
        continue
    }
    New-Item -ItemType Directory -Force -Path (Split-Path $dst) | Out-Null
    Copy-Item -Recurse -Force $src $dst
    Write-Host "ok: $rel"
}
