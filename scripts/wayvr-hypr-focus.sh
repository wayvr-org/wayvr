#!/bin/bash
set -e

REPO_ROOT="/home/taylor/dev/open-source/real-wayvr"
WAYVRCTL="$REPO_ROOT/target/debug/wayvrctl"

if [ ! -f "$WAYVRCTL" ]; then
    echo "wayvrctl not found, building..."
    cd "$REPO_ROOT"
    cargo build --package wayvrctl
fi

active_window=$(hyprctl -j activewindow)

if [ -z "$active_window" ] || [ "$active_window" = "{}" ]; then
    echo "No active window found in Hyprland"
    exit 1
fi

monitor_id=$(echo "$active_window" | jq -r '.monitor')

if [ -z "$monitor_id" ] || [ "$monitor_id" = "null" ]; then
    echo "Could not get active monitor id"
    exit 1
fi

monitor_name=$(hyprctl -j monitors | jq -r --argjson monitor_id "$monitor_id" '.[] | select(.id == $monitor_id) | .name' | head -n 1)

if [ -z "$monitor_name" ] || [ "$monitor_name" = "null" ]; then
    echo "Could not resolve monitor name for id $monitor_id"
    exit 1
fi

echo "Toggling focused screen for active monitor: $monitor_name"
exec "$WAYVRCTL" screen-focus-toggle "$monitor_name"
