#!/usr/bin/env bash
# Runtime tests for the engine in a headless environment (no hardware GPU).
#
# Uses Mesa lavapipe (a CPU Vulkan driver) so the real Vulkan pipeline runs, and
# Xvfb (a virtual X display) so the winit-based editor can open a window. This
# is how the GPU-dependent tests and the editor smoke can be exercised in CI or
# a container.
#
# Usage:
#   bash scripts/run-runtime-tests.sh            # tests only
#   bash scripts/run-runtime-tests.sh --editor   # tests + an editor smoke run
set -euo pipefail
cd "$(dirname "$0")/.."

# 1. Dependencies (idempotent). Needs sudo/apt on Debian/Ubuntu.
need=(mesa-vulkan-drivers vulkan-tools libxkbcommon-x11-0 xvfb)
missing=()
dpkg -s "${need[@]}" >/dev/null 2>&1 || {
  for p in "${need[@]}"; do dpkg -s "$p" >/dev/null 2>&1 || missing+=("$p"); done
}
if [ "${#missing[@]}" -gt 0 ]; then
  echo "Installing: ${missing[*]}"
  sudo apt-get update -qq && sudo apt-get install -y -qq "${missing[@]}"
fi

# 2. Point the Vulkan loader at lavapipe; lavapipe has no ray-tracing, so disable
#    the engine's RT path.
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
export DFE_RT=0

echo "== Vulkan device =="
vulkaninfo --summary 2>/dev/null | grep -E "deviceName|driverName" | head -2 || true

# 3. GPU-dependent tests (each skips gracefully if no device is found).
echo "== cargo test --test ui_runtime =="
cargo test --test ui_runtime
echo "== cargo test --test render_animated_glb =="
cargo test --test render_animated_glb

# 4. Optional: launch the editor under a virtual display for a few seconds and
#    confirm it initializes Vulkan + renders without crashing. Exit 124 (timeout)
#    means it ran the render loop successfully.
if [ "${1:-}" = "--editor" ]; then
  echo "== editor smoke (Xvfb + lavapipe, 15s) =="
  cargo build --bin editor
  set +e
  xvfb-run -a -s "-screen 0 1280x800x24" timeout 15 ./target/debug/editor
  code=$?
  set -e
  if [ "$code" = "124" ]; then
    echo "editor smoke OK (ran until timeout)"
  else
    echo "editor smoke exited with $code"; exit "$code"
  fi
fi

echo "runtime tests done."
