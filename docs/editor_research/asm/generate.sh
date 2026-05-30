#!/usr/bin/env bash
# Regenerates every assembly listing embedded in ../../../EDITOR_research.md.
# Reproducible companion to the repo's benchmark culture.
#
# Usage:  bash generate.sh
# Output: *.s (Intel syntax) next to each snippet, plus versions.txt.
set -euo pipefail
cd "$(dirname "$0")"

OPT="-O -C opt-level=3 -C target-cpu=x86-64-v3 --emit asm \
     -C llvm-args=-x86-asm-syntax=intel --crate-type=lib"

echo "# Generated $(date -u +%Y-%m-%dT%H:%M:%SZ)" > versions.txt
rustc --version >> versions.txt
echo "flags: $OPT" >> versions.txt

for f in exp1_halfedge exp2_transform exp3_normals exp4_select_upload exp5_subdiv; do
  if [ -f "$f.rs" ]; then
    echo "==> $f"
    # shellcheck disable=SC2086
    rustc $OPT "$f.rs" -o "$f.s"
  fi
done

echo "Done. Listings: $(ls *.s 2>/dev/null | tr '\n' ' ')"
