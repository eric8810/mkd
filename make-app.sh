#!/bin/sh
# 打包 mkd 为 macOS 原生 .app bundle
# 用法: ./make-app.sh [release|debug]
set -e
cd "$(dirname "$0")"

PROFILE="${1:-release}"
SRC="target/$PROFILE/mkd"
APP="dist/mkd.app"

if [ ! -x "$SRC" ]; then
  echo "未找到 $SRC，先执行: cargo build --$PROFILE"
  exit 1
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$SRC" "$APP/Contents/MacOS/mkd"
cp dist/Info.plist "$APP/Contents/Info.plist"
chmod +x "$APP/Contents/MacOS/mkd"
codesign --force --sign - "$APP" 2>/dev/null || true

echo "已生成 $APP"
du -sh "$APP"
