#!/bin/sh
# アプリのアイコン assets/icon/fude.ico を assets/icon/fude.svg と fude-small.svg から作る（ImageMagick 7 の magick が要る）。
# アイコンの SVG を変えたときに実行し、できた fude.ico もコミットする。
set -eu
cd "$(dirname "$0")/.."
SVG=assets/icon/fude.svg
# 16〜32 px は線を太くした小さい用から作る（細い線がにじんで消えるため）。
SMALL=assets/icon/fude-small.svg
OUT=assets/icon/fude.ico
# Windows の magick は /tmp のような MSYS のパスを読めないので、リポジトリの中の相対パスを使う。
TMP=target/gen-icon
rm -rf "$TMP" && mkdir -p "$TMP"
trap 'rm -rf "$TMP"' EXIT
for s in 256 64 48 40 32 24 20 16; do
    src="$SVG"
    [ "$s" -le 32 ] && src="$SMALL"
    magick -background none -density 384 "$src" -resize "${s}x${s}" "PNG32:$TMP/$s.png"
done
magick "$TMP/256.png" "$TMP/64.png" "$TMP/48.png" "$TMP/40.png" "$TMP/32.png" "$TMP/24.png" "$TMP/20.png" "$TMP/16.png" "$OUT"
echo "$OUT"
