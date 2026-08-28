#!/usr/bin/env bash
set -euo pipefail

root="${IMG2OMEZARR_REAL_FIXTURE_DIR:-fixtures/real-world}"

if [[ ! -d "$root" ]]; then
    echo "fixture directory not found: $root" >&2
    echo "run scripts/prepare_real_world_fixtures.sh first" >&2
    exit 1
fi

found=0
for fixture in "$root"/*; do
    if [[ ! -f "$fixture" ]]; then
        continue
    fi
    found=1
    echo
    echo "## $fixture"
    cargo run -p img2omezarr --features cli --bin img2omezarr -- \
        inspect "$fixture" --format markdown
done

if [[ "$found" -eq 0 ]]; then
    echo "no fixture files found under $root" >&2
    exit 1
fi
