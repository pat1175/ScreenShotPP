#!/usr/bin/env bash
# Télécharge un ffmpeg statique par plateforme dans app/binaries/ sous le
# nom attendu par external-binaries de cargo-packager : ffmpeg-<target-triple>[.exe].
# Usage : scripts/fetch-ffmpeg.sh <windows|macos-arm>
# NOTE: vérifier/mettre à jour les URLs épinglées au moment de l'implémentation.
set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)/app/binaries"
mkdir -p "$DIR"

case "${1:?usage: fetch-ffmpeg.sh <windows|macos-arm>}" in
  windows)
    OUT="$DIR/ffmpeg-x86_64-pc-windows-msvc.exe"
    [ -f "$OUT" ] && { echo "déjà présent: $OUT"; exit 0; }
    # Les URLs "latest" ne peuvent pas être validées par checksum ; on accepte cette
    # limite de confiance (ffmpeg s'exécute comme processus séparé).
    URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-win64-gpl-8.1.zip"
    curl -fsSL "$URL" -o /tmp/ffmpeg-win.zip
    unzip -j -o /tmp/ffmpeg-win.zip '*/bin/ffmpeg.exe' -d "$DIR"
    mv "$DIR/ffmpeg.exe" "$OUT"
    rm -f /tmp/ffmpeg-win.zip
    ;;
  macos-arm)
    OUT="$DIR/ffmpeg-aarch64-apple-darwin"
    [ -f "$OUT" ] && { echo "déjà présent: $OUT"; exit 0; }
    # Les URLs "latest" ne peuvent pas être validées par checksum ; on accepte cette
    # limite de confiance (ffmpeg s'exécute comme processus séparé).
    URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip"
    curl -fsSL "$URL" -o /tmp/ffmpeg-mac.zip
    unzip -j -o /tmp/ffmpeg-mac.zip ffmpeg -d "$DIR"
    mv "$DIR/ffmpeg" "$OUT"
    chmod +x "$OUT"
    rm -f /tmp/ffmpeg-mac.zip
    ;;
  *) echo "cible inconnue: $1" >&2; exit 1 ;;
esac
# Trace la version résolue (les URLs sont des pointeurs "latest") ; non fatal si
# le binaire ne tourne pas sur cet hôte (ex. dev Linux).
"$OUT" -version 2>/dev/null | head -n1 || echo "version ffmpeg non détectable sur cet hôte"
echo "ffmpeg prêt: $OUT"
