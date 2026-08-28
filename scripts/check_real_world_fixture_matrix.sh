#!/usr/bin/env bash
set -euo pipefail

matrix="${1:-docs/validation.md}"

if [[ ! -f "$matrix" ]]; then
    echo "matrix file not found: $matrix" >&2
    exit 1
fi

required_formats=(
    "OME-TIFF"
    "ND2"
    "CZI"
    "LIF"
    "SVS or whole-slide pyramid"
    "HCS/plate dataset"
)

for format in "${required_formats[@]}"; do
    if ! grep -F "| $format |" "$matrix" >/dev/null; then
        echo "missing fixture matrix row: $format" >&2
        exit 1
    fi
done

if grep -E "To be measured|Needed|not downloaded|local validation use only until recorded" "$matrix" >/dev/null; then
    echo "fixture matrix still contains placeholder or unverified fields" >&2
    grep -nE "To be measured|Needed|not downloaded|local validation use only until recorded" "$matrix" >&2
    exit 1
fi

echo "fixture matrix is complete: $matrix"
