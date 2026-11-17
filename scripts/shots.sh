#!/usr/bin/env bash
# 用 Edge 无头模式给各 mock 场景截图（浅色 / 深色），输出到 $1
set -u
OUT="$1"
mkdir -p "$OUT"
EDGE="/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"
for scene in ok portBusy authMissing onboard; do
  for theme in light dark; do
    flag=""
    [ "$theme" = dark ] && flag="--blink-settings=preferredColorScheme=0" || flag="--blink-settings=preferredColorScheme=1"
    "$EDGE" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=2 --window-size=480,640 \
      $flag --virtual-time-budget=3000 --screenshot="$(cygpath -w "$OUT")\\$scene-$theme.png" \
      "http://127.0.0.1:4173/?mock=$scene" >/dev/null 2>&1
  done
done
ls "$OUT"
