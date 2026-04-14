#!/bin/bash
set -e

WAYVRCTL="${WAYVRCTL:-wayvrctl}"
STATE_DIR="/tmp/wayvr-hypr-focus"
PID_FILE="$STATE_DIR/watch.pid"
SCREEN_FILE="$STATE_DIR/screen_name"
SOCKET_PATH="${XDG_RUNTIME_DIR}/hypr/${HYPRLAND_INSTANCE_SIGNATURE}/.socket2.sock"

mkdir -p "$STATE_DIR"

run_focus_command() {
    local refresh_flag="$1"
    local expected_screen="${2:-}"

    active_window=$(hyprctl -j activewindow)

    if [ -z "$active_window" ] || [ "$active_window" = "{}" ]; then
        return 1
    fi

    monitor_id=$(echo "$active_window" | jq -r '.monitor')

    if [ -z "$monitor_id" ] || [ "$monitor_id" = "null" ]; then
        return 1
    fi

    monitor_name=$(hyprctl -j monitors | jq -r --argjson monitor_id "$monitor_id" '.[] | select(.id == $monitor_id) | .name' | head -n 1)
    monitor_json=$(hyprctl -j monitors | jq -c --argjson monitor_id "$monitor_id" '.[] | select(.id == $monitor_id)' | head -n 1)

    if [ -z "$monitor_name" ] || [ "$monitor_name" = "null" ]; then
        return 1
    fi

    if [ -n "$expected_screen" ] && [ "$monitor_name" != "$expected_screen" ]; then
        return 1
    fi

    target_x=$(jq -n \
        --argjson active "$active_window" \
        --argjson monitor "$monitor_json" \
        '(((($active.at[0] + ($active.size[0] / 2)) - $monitor.x) / $monitor.width) | if . < 0 then 0 elif . > 1 then 1 else . end)')

    target_y=$(jq -n \
        --argjson active "$active_window" \
        --argjson monitor "$monitor_json" \
        '(((($active.at[1] + ($active.size[1] / 2)) - $monitor.y) / $monitor.height) | if . < 0 then 0 elif . > 1 then 1 else . end)')

    crop_x=$(jq -n \
        --argjson active "$active_window" \
        --argjson monitor "$monitor_json" \
        '((($active.at[0] - $monitor.x) / $monitor.width) | if . < 0 then 0 elif . > 1 then 1 else . end)')

    crop_y=$(jq -n \
        --argjson active "$active_window" \
        --argjson monitor "$monitor_json" \
        '((($active.at[1] - $monitor.y) / $monitor.height) | if . < 0 then 0 elif . > 1 then 1 else . end)')

    crop_w=$(jq -n \
        --argjson active "$active_window" \
        --argjson monitor "$monitor_json" \
        '((($active.size[0]) / $monitor.width) | if . < 0.02 then 0.02 elif . > 1 then 1 else . end)')

    crop_h=$(jq -n \
        --argjson active "$active_window" \
        --argjson monitor "$monitor_json" \
        '((($active.size[1]) / $monitor.height) | if . < 0.02 then 0.02 elif . > 1 then 1 else . end)')

    "$WAYVRCTL" screen-focus-at $refresh_flag \
        --crop-x "$crop_x" --crop-y "$crop_y" --crop-w "$crop_w" --crop-h "$crop_h" \
        "$monitor_name" "$target_x" "$target_y"

    printf '%s' "$monitor_name" > "$SCREEN_FILE"
}

event_stream() {
    python3 - "$SOCKET_PATH" <<'PY'
import socket
import sys

sock_path = sys.argv[1]
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(sock_path)
buf = b""

while True:
    data = sock.recv(4096)
    if not data:
        break
    buf += data
    while b"\n" in buf:
        line, buf = buf.split(b"\n", 1)
        sys.stdout.write(line.decode("utf-8", "replace") + "\n")
        sys.stdout.flush()
PY
}

should_refresh_event() {
    case "$1" in
        activewindowv2*|activewindow*|fullscreen*|movewindow*|movewindowv2*|changefloatingmode*|openwindow*|closewindow*)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

if [ "${1:-}" = "--watch" ]; then
    watched_screen="$2"
    while [ -f "$PID_FILE" ] && [ "$(cat "$PID_FILE")" = "$$" ]; do
        event_stream | while IFS= read -r line; do
            if [ ! -f "$PID_FILE" ] || [ "$(cat "$PID_FILE")" != "$$" ]; then
                break
            fi

            if should_refresh_event "$line"; then
                run_focus_command --refresh-only "$watched_screen" || true
            fi
        done
        sleep 0.2
    done
    exit 0
fi

if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    watched_screen="$(cat "$SCREEN_FILE" 2>/dev/null || true)"
    kill "$(cat "$PID_FILE")" 2>/dev/null || true
    rm -f "$PID_FILE" "$SCREEN_FILE"
    if [ -n "$watched_screen" ]; then
        exec "$WAYVRCTL" screen-focus-toggle "$watched_screen"
    fi
    exit 0
fi

run_focus_command "" || {
    echo "No active window found"
    exit 1
}

nohup "$0" --watch "$(cat "$SCREEN_FILE")" >/dev/null 2>&1 &
printf '%s' "$!" > "$PID_FILE"
