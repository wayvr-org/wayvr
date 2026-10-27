#!/bin/sh
set -eu

case "$(uname -m)" in
  x86_64|amd64)
    APPIMAGE_ARCH=x86_64
    ;;
  aarch64|arm64)
    APPIMAGE_ARCH=aarch64
    ;;
  *)
    echo "Unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

LINUXDEPLOY="linuxdeploy-${APPIMAGE_ARCH}.AppImage"

echo "Preparing AppImage environment for ${APPIMAGE_ARCH}"

sudo apt-get update
sudo apt-get install -y fuse cmake pkg-config fontconfig libasound2-dev libxkbcommon-dev libxkbcommon-x11-0 libxkbcommon-x11-dev \
  libopenxr-dev libfontconfig-dev libdbus-1-dev libpipewire-0.3-0 libpipewire-0.3-dev libspa-0.2-dev libx11-6 libxext6 libxrandr2 \
  libx11-dev libxext-dev libxrandr-dev libopenvr-dev libopenvr-api1t64 libwayland-dev libegl-dev libxcb-glx0 libxcb-glx0-dev \
  dav1d libdav1d-dev libvulkan-dev glslc vulkan-tools libudev-dev libudev1 libinput-dev libinput10 spirv-headers

rustup update

if [ -n "${APPDIR:-}" ]; then
  if [ ! -f "$LINUXDEPLOY" ]; then
    wget -q "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/${LINUXDEPLOY}"
  fi
  chmod +x "$LINUXDEPLOY"

  if [ -d "$APPDIR" ]; then
    rm -rf "$APPDIR"
  fi
  mkdir -p "$APPDIR/usr/bin"
fi
