#!/bin/sh

sudo apt install \
    xwayland \
    libxcb1-dev \
    libxcb-cursor-dev \
    clang \
    pkg-config

git clone https://github.com/Supreeeme/xwayland-satellite.git
cd xwayland-satellite

cargo build --release
chmod +x ../target/release/xwayland-satellite
cd ..

cp xwayland-satellite/target/release/wayvr ${APPDIR}/usr/bin
