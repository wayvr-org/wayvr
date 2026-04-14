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
    echo "No active window found"
    exit 1
fi

monitor_id=$(echo "$active_window" | jq -r '.monitor')
monitor_name=$(hyprctl -j monitors | jq -r --argjson monitor_id "$monitor_id" '.[] | select(.id == $monitor_id) | .name' | head -n 1)
monitor_json=$(hyprctl -j monitors | jq -c --argjson monitor_id "$monitor_id" '.[] | select(.id == $monitor_id)' | head -n 1)

if [ -z "$monitor_name" ] || [ "$monitor_name" = "null" ]; then
    echo "Could not resolve active monitor"
    exit 1
fi

target_x=$(jq -n \
    --argjson active "$active_window" \
    --argjson monitor "$monitor_json" \
    '(((($active.at[0] + ($active.size[0] / 2)) - $monitor.x) / $monitor.width) | if . < 0 then 0 elif . > 1 then 1 else . end)')

target_y=$(jq -n \
    --argjson active "$active_window" \
    --argjson monitor "$monitor_json" \
    '(((($active.at[1] + ($active.size[1] / 2)) - $monitor.y) / $monitor.height) | if . < 0 then 0 elif . > 1 then 1 else . end)')

echo "Current wayvrctl screen focus commands:"
"$WAYVRCTL" screen-focus-toggle --help
echo
"$WAYVRCTL" screen-focus-at --help
echo
echo "Toggling screen focus for active monitor: $monitor_name"
"$WAYVRCTL" screen-focus-toggle "$monitor_name"
echo
echo "Refreshing screen focus at target: monitor=$monitor_name x=$target_x y=$target_y"
"$WAYVRCTL" screen-focus-at --refresh-only "$monitor_name" "$target_x" "$target_y"
