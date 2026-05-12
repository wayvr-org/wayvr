#!/bin/sh

sudo apt-get update
sudo apt-get install -y fuse cmake pkg-config fontconfig libasound2-dev libxkbcommon-dev libxkbcommon-x11-0 libxkbcommon-x11-dev libopenxr-dev libfontconfig-dev libdbus-1-dev libpipewire-0.3-0 libpipewire-0.3-dev libspa-0.2-dev libx11-6 libxext6 libxrandr2 libx11-dev libxext-dev libxrandr-dev libopenvr-dev libopenvr-api1 libwayland-dev libegl-dev libxcb-glx0 libxcb-glx0-dev
rustup update

if [ "$APPDIR" != "" ]; then
  test -f linuxdeploy-x86_64.AppImage || wget -q "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"
  chmod +x linuxdeploy-x86_64.AppImage

  test -d ${APPDIR} && rm -rf ${APPDIR}
  mkdir -p ${APPDIR}/usr/bin
fi
