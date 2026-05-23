#!/usr/bin/env sh
# Bootstrap script for dumpster_fire_engine dev environment.
# Installs Rust if absent, then delegates to the Rust setup binary which
# handles LLVM 18, Vulkan, CMake, Valgrind, iai-callgrind-runner, and
# writes .env.toolchain with the correct LLVM_SYS_180_PREFIX.
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── 1. Ensure Rust / cargo is available ───────────────────────────────────────
if ! command -v cargo >/dev/null 2>&1; then
    echo "[setup] cargo not found — installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path
    # Source for this shell session only; the user's profile gets updated by rustup
    # shellcheck disable=SC1091
    . "${HOME}/.cargo/env"
fi

# ── 2. Run the Rust setup binary ──────────────────────────────────────────────
# --quiet suppresses Cargo's own compile noise; the binary's output is the UI.
exec cargo run \
    --manifest-path "${SCRIPT_DIR}/tools/setup/Cargo.toml" \
    --quiet \
    -- "$@"
