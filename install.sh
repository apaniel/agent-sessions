#!/bin/bash

APP_NAME="Claude Sessions"
DMG_PATH="src-tauri/target/release/bundle/dmg/Claude Sessions_0.1.0_aarch64.dmg"
APP_PATH="/Applications/${APP_NAME}.app"
MOUNT_POINT="/Volumes/${APP_NAME}"

echo "🔄 Installing ${APP_NAME}..."

# Kill running instance
if pgrep -x "${APP_NAME}" > /dev/null; then
    echo "⏹️  Stopping running instance..."
    pkill -x "${APP_NAME}"
    sleep 1
fi

# Unmount if already mounted
if [ -d "${MOUNT_POINT}" ]; then
    echo "📤 Unmounting existing DMG..."
    hdiutil detach "${MOUNT_POINT}" -quiet
fi

# Mount DMG
echo "📀 Mounting DMG..."
hdiutil attach "${DMG_PATH}" -quiet

# Remove old app if exists
if [ -d "${APP_PATH}" ]; then
    echo "🗑️  Removing old version..."
    rm -rf "${APP_PATH}"
fi

# Copy new app
echo "📦 Installing new version..."
cp -R "${MOUNT_POINT}/${APP_NAME}.app" /Applications/

# Unmount DMG
echo "📤 Unmounting DMG..."
hdiutil detach "${MOUNT_POINT}" -quiet

# Launch app
echo "🚀 Launching ${APP_NAME}..."
open "${APP_PATH}"

echo "✅ Done!"
