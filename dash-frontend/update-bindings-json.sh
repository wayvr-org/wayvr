#!/usr/bin/env bash
set -euo pipefail

readonly ALLOWED_PROFILES="$(
  cat <<'EOF'
/interaction_profiles/hp/mixed_reality_controller
/interaction_profiles/htc/vive_controller
/interaction_profiles/htc/vive_cosmos_controller
/interaction_profiles/htc/vive_focus3_controller
/interaction_profiles/khr/simple_controller
/interaction_profiles/ml/ml2_controller
/interaction_profiles/microsoft/motion_controller
/interaction_profiles/mndx/flipvr
/interaction_profiles/mndx/pssense_controller_mndx
/interaction_profiles/oculus/touch_controller
/interaction_profiles/oppo/mr_controller_oppo
/interaction_profiles/samsung/odyssey_controller
/interaction_profiles/valve/index_controller
EOF
)"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
output_dir="${script_dir}/assets"
output_json="${output_dir}/bindings.json"
output_lz4="${output_json}.lz4"

repo_url="https://gitlab.freedesktop.org/monado/monado.git"
repo_branch="main"
bindings_path="src/xrt/auxiliary/bindings/bindings.json"

tmpdir="$(mktemp -d)"
tmp_output=""

cleanup() {
  rm -rf "$tmpdir"

  if [[ -n "$tmp_output" && -f "$tmp_output" ]]; then
    rm -f "$tmp_output"
  fi
}
trap cleanup EXIT

command -v git >/dev/null || { echo "git is required" >&2; exit 1; }
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
command -v lz4 >/dev/null || { echo "lz4 is required" >&2; exit 1; }

git clone \
  --depth 1 \
  --branch "$repo_branch" \
  --filter=blob:none \
  --sparse \
  "$repo_url" \
  "$tmpdir/monado"

git -C "$tmpdir/monado" sparse-checkout set --no-cone "$bindings_path"

input_json="$tmpdir/monado/$bindings_path"

mkdir -p "$output_dir"
tmp_output="$(mktemp "${output_json}.tmp.XXXXXX")"

jq_filter='
  ($allowed_lines
    | split("\n")
    | map(sub("\r$"; ""))
    | map(select(length > 0))
    | reduce .[] as $key ({}; .[$key] = true)
  ) as $allowed
  | .profiles |= with_entries(select($allowed[.key] == true))
'

jq -c \
  --arg allowed_lines "$ALLOWED_PROFILES" \
  "$jq_filter" \
  "$input_json" > "$tmp_output"

mv "$tmp_output" "$output_json"
tmp_output=""

lz4 -f --rm "$output_json" "$output_lz4"

echo "Wrote compressed filtered bindings to: $output_lz4"
