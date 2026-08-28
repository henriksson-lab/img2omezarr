#!/usr/bin/env bash
set -euo pipefail

root="${IMG2OMEZARR_REAL_FIXTURE_DIR:-fixtures/real-world}"
mkdir -p "$root"

download_fixture() {
    local name="$1"
    local url="$2"
    local target="$root/$name"

    if [[ -z "$url" ]]; then
        return
    fi

    if [[ -e "$target" ]]; then
        echo "exists: $target"
        return
    fi

    echo "downloading $url"
    curl -L --fail --output "$target" "$url"
}

download_fixture "ome-tiff.fixture" "${OME_TIFF_FIXTURE_URL:-}"
download_fixture "nd2.fixture" "${ND2_FIXTURE_URL:-}"
download_fixture "czi.fixture" "${CZI_FIXTURE_URL:-}"
download_fixture "lif.fixture" "${LIF_FIXTURE_URL:-}"
download_fixture "svs.fixture" "${SVS_FIXTURE_URL:-}"
download_fixture "hcs.fixture" "${HCS_FIXTURE_URL:-}"

cat <<EOF
Prepared fixture directory: $root

Next steps:
1. Review the source license for every downloaded fixture.
2. Record dimensions and metadata in docs/validation.md:
   cargo run -p img2omezarr --features cli --bin img2omezarr -- inspect $root/<fixture-file> --format markdown
3. Run the real-world smoke test once tests/real_world.rs exists.
EOF
