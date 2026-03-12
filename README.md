![WayVR splash screen header](https://github.com/wlx-team/wayvr/blob/guide/wayvr-readme-header.webp?raw=true)

# WayVR (previously WlxOverlay-S)

A lightweight OpenXR/OpenVR overlay for Wayland and X11 desktops.

WayVR lets you access your desktop screens while in VR, and even launch apps directly in VR.

In comparison to similar overlays, WayVR aims to run alongside VR games and experiences while having as little performance impact as possible. The UI appearance and rendering techniques are kept as simple and efficient as possible, while still allowing a high degree of customizability.

![Screenshot of WayVR being used as an OpenXR home environment](https://github.com/wlx-team/wayvr/blob/guide/wayvr-readme-screenshot.webp?raw=true)

## Join the Linux VR Community

We are available on either **Discord** or **Matrix space**:

[![LVRA Discord](https://img.shields.io/discord/1065291958328758352?style=for-the-badge&logo=discord)](https://discord.gg/EHAYe3tTYa) [![LVRA Matrix](https://img.shields.io/matrix/linux-vr-adventures:matrix.org?logo=matrix&style=for-the-badge)](https://matrix.to/#/#linux-vr-adventures:matrix.org)

Questions/issues specific to WayVR will be handled in the `wayvr` chat room. Feel free to ask anything.

## Setup

### Installation

There are multiple ways to install WayVR:

1. AppImage: Download from [Releases](https://github.com/wlx-team/wayvr/releases)
1. AUR package: [wayvr](https://aur.archlinux.org/packages/wayvr) or [wayvr-git](https://aur.archlinux.org/packages/wayvr-git) 
1. Nix package: [wayvr](https://search.nixos.org/packages?channel=unstable&show=wayvr&query=wayvr) or [unstable package from nixpkgs-xr](https://github.com/nix-community/nixpkgs-xr)
1. [Building from source](https://github.com/wlx-team/wayvr/wiki/Building-from-Source).

### General Setup

1. Start Monado, WiVRn or SteamVR.
1. Run the overlay

**Note:** If you are using Monado or WiVRn, no additional setup steps are required for Flatpak Steam compatibility—most people use WayVR seamlessly with Monado/WiVRn.

### SteamVR via Steam Flatpak

For users specifically running **SteamVR via Steam Flatpak**, follow these steps:

1. Grab the latest AppImage from [Releases](https://github.com/wlx-team/wayvr/releases).
1. `WayVR-*.AppImage --appimage-extract`
1. `chmod +x squashfs-root/AppRun`
1. Move the newly created `squashfs-root` folder to a location accessible by the Steam Flatpak.
1. `flatpak override com.valvesoftware.Steam --user --filesystem=xdg-run/pipewire-0/:rw`
1. Restart Steam.
1. Start SteamVR.
1. `flatpak run --command='/path/to/squashfs-root/AppRun' com.valvesoftware.Steam`

## First Start

**When the screen share pop-up appears, check your notifications or the terminal and select the screens in the order it requests.**

In case screens were selected in the wrong order:

- Go to Settings and press `Clear PipeWire tokens` and then `Restart software`
- Pay attention to your notifications, which tell you in which order to pick the screens.
- If notifications don't show, try starting WayVR from the terminal and look for instructions in there.

**WiVRn users**: Select WayVR from the `Application` drop-down. If there's no such entry, select `Custom` and browse to your WayVR executable or AppImage.

**Envision users**: Go to the Plugins menu and select the WayVR plugin. This will download and run the AppImage version of the overlay.
To run a standalone installation (for instance, from the AUR), create a bash script containing `wayvr --openxr --show` and then set this bash script as a custom Envision plugin.

This will show a home environment with headset passthrough enabled by default or a [customizable background](https://github.com/wlx-team/wayvr/wiki/OpenXR-Skybox)!

**SteamVR users**: WayVR will register itself for auto-start, so there is no need to start it every time. Disclaimer: SteamVR will sometimes disregard this and not start WayVR anyway.

**Please continue reading the guide below.**

## Getting Started

### Working Set

The working set consists of all currently selected overlays: screens, mirrors, keyboard, etc.

The working set appears in front of the headset when shown, and can be re-centered by hiding and showing again.

Show and hide the working set using:

- Non-vive controller: double-tap B or Y on the left controller.
- Vive controller: double-tap the menu button on the left controller (for SteamVR, the `showhide` binding must be bound)

See the [bindings](#default-bindings) section on how to grab, move and resize overlay windows.

### Pointer Modes AKA Laser Colors

Much of the functionality in WayVR depends on what color of laser is used to interact with a UI element. \
Using the default settings, there are 3 modes:

- Regular Mode: Blue laser
- Right-click Mode: Orange laser
- Middle-click Mode: Purple laser

Please see the bindings section below on how to activate these modes.

The guide here uses the colors for ease of getting started.

### The watch

Check your left wrist for the watch. The watch is the primary tool for controlling the app.

The top of the watch shows device batteries, and the bottom shows your overlay controls.

Enter edit mode (the leftmost button at the bottom) to edit your overlay sets.

While in edit mode, the watch can also be grabbed and passed between your hands.

After grabbing, the watch will automatically attach to the hand that's opposite from the one that held it.

In edit mode, try hovering over other overlays to see their advanced options!

### The screens

Hovering a pointer over a screen will move the mouse. If more than one pointer is hovering over a screen, the pointer that was last used to click will take precedence.

The click type depends on the laser color:

- Blue laser: Left click
- Orange laser: Right click
- Purple laser: Middle click
- Stick up/down: Scroll wheel

### The keyboard

Typing

- Use the BLUE laser when typing regularly.
- While using the ORANGE laser, all keystrokes will have SHIFT applied.
- Purple laser is customizable via the settings, no modifier by default.

**Modifier Keys are sticky**. They will remain pressed until eiter:
- a non-modifier key is pressed
- the modifier is toggled off by clicking again
- the keyboard is hidden (including via show-hide)

### Default Bindings

![Index Controller Bindings](https://github.com/wlx-team/wayvr/blob/guide/wlx-index.png)

![Touch Controller Bindings](https://github.com/wlx-team/wayvr/blob/guide/wlx-oculus.png)


### Changing Bindings

SteamVR: Simply change the bindings from the SteamVR bindings section. If WayVR doesn't show up on the list, select any other title and then press back on the top left. (SteamVR is weird like that sometimes)

OpenXR (Monado/WiVRn): See [Wiki: OpenXR Bindings](https://github.com/wlx-team/wayvr/wiki/OpenXR-Bindings)

If your controllers are not supported, please reach out. \
We would like to work with you and include additional bindings.

## Customization

See these relevant wiki pages:
- For all available config options, check [Wiki: Configuration](https://github.com/wlx-team/wayvr/wiki/Configuration)
- Looking to customize look & feel, or add functionality? See [Wiki: Customization](https://github.com/wlx-team/wayvr/wiki/Customization)
- Looking to change the OpenXR background? See [Wiki: OpenXR Skybox](https://github.com/wlx-team/wayvr/wiki/OpenXR-Skybox)

## Troubleshooting

When an error is detected, we often print tips for fixing it into the log file.

Logs will be at `/tmp/wayvr.log` for most distros.

Check [here](https://github.com/wlx-team/wayvr/wiki/Troubleshooting) for tips.

## Known Issues

### Mouse is not where it should be

If the mouse is moving on a completely different screen, the screens were likely selected in the wrong order:

- Go to Settings and press `Clear PipeWire tokens` and then `Restart software`
- Pay attention to your notifications, which tell you in which order to pick the screens.
- If notifications don't show, try starting WayVR from the terminal and look for instructions in there.

COSMIC desktop:

- Due to limitations with COSMIC, the mouse can only move on a single display.

X11 users:

- Might be dealing with a [Phantom Monitor](https://wiki.archlinux.org/title/Xrandr#Disabling_phantom_monitor).
- DPI scaling is not supported and will mess with the mouse.
- Upright screens are not supported and will mess with the mouse.

### Screens are blank or black or frozen on SteamVR 2.14+

As of SteamVR version 2.14.x, PipeWire capture no longer works when using Steam Link.

We're unable to completely troubleshoot how and why SteamVR interferes with PipeWire, so consider the following workarounds for the time being:

- Use another streamer, such as WiVRn or ALVR. Note that SteamVR on Linux is considered experimental by Valve themselves [disclaimer](https://github.com/ValveSoftware/SteamVR-for-Linux?tab=readme-ov-file#steamvr-release-notes-and-known-issues).
- If your desktop [supports ScreenCopy](https://wayland.app/protocols/wlr-screencopy-unstable-v1#compositor-support), go to Settings and set `Wayland capture method` to `ScreenCopy`
- If your desktop has an X11 mode, try using that

### Modifiers get stuck

Hiding the keyboard will unpress all of its buttons. Alternatively, go to Settings and use the `Restart software` button.

### X11 limitations

- X11 capture can generally seem slow. This is because zero-copy GPU capture is not supported on the general X11 desktop. Consider trying Wayland.
- DPI scaling is not supported and may cause the mouse to not follow the laser properly.
- Upright screens are not supported and can cause the mouse to not follow the laser properly.
- Screen changes (connecting/disconnecting a display, resolution changes, etc) are not handled at runtime. Restart the overlay for these to take effect.
