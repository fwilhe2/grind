#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# How big is a container image? The README calls the CLI image "about as small as
# one gets" and the web image "no shell, no package manager" — both are claims
# about size, so CI measures them instead of trusting them.
#
# Two numbers, because they answer different questions and neither implies the
# other:
#
#   local  — what the daemon that just built it reports. Available before a push,
#            so it is the number a pull request gets. What it *means* depends on
#            the image store: the sum of the uncompressed layers on the classic
#            overlay2 driver the runners use, the compressed content size under
#            the containerd snapshotter (Fedora's default, so a local run of this
#            prints a smaller figure than CI does for the same image). Hence the
#            table says where the number came from rather than claiming either.
#   remote — the manifest's own layer sizes, which are compressed: what a
#            `docker pull` actually transfers, whatever store either end uses.
#            Only exists once pushed, and walks a multi-arch tag's index one
#            platform at a time.
#
# Usage:
#   image-size.sh local  ghcr.io/fwilhe2/grind:latest-amd64
#   image-size.sh remote ghcr.io/fwilhe2/grind:latest
#
# Every row goes to stdout and, when running under Actions, to the job summary.

set -euo pipefail

mode=${1:-}
shift || true

if [ -z "$mode" ] || [ "$#" -eq 0 ]; then
    echo "usage: $0 <local|remote> <image>..." >&2
    exit 2
fi

summary=${GITHUB_STEP_SUMMARY:-/dev/null}

case "$mode" in
    local)
        what="built image, as \`docker image inspect\` reports it"
        ;;
    remote)
        what="pull size, the compressed layers a client transfers"
        ;;
    *)
        echo "$0: unknown mode '$mode' (expected 'local' or 'remote')" >&2
        exit 2
        ;;
esac

# The repository part of a reference, with any tag or digest removed — needed to
# ask about one platform of a multi-arch tag by digest. A colon is only a tag
# separator when it comes after the last slash; before it, it is a registry port.
repo_of() {
    local ref=${1%@*}
    case "${ref##*/}" in
    *:*) ref=${ref%:*} ;;
    esac
    printf '%s' "$ref"
}

human() { numfmt --to=iec-i --suffix=B --format='%.1f' "$1"; }

row() { # row <name> <bytes>
    printf '| %s | %s | %s |\n' "$1" "$(human "$2")" "$2" | tee -a "$summary"
}

# The config blob is counted with the layers: it is pulled too, and leaving it
# out would report a number no client ever transfers.
manifest_bytes() { jq '[.config.size] + [.layers[].size] | add'; }

{
    printf '### Image size — %s\n\n' "$what"
    printf '| Image | Size | Bytes |\n|---|---:|---:|\n'
} | tee -a "$summary"

for ref in "$@"; do
    case "$mode" in
    local)
        row "$ref" "$(docker image inspect --format '{{.Size}}' "$ref")"
        ;;
    remote)
        raw=$(docker buildx imagetools inspect --raw "$ref")
        if jq -e 'has("manifests")' >/dev/null <<<"$raw"; then
            # An index: one row per real platform. `unknown/unknown` entries are
            # buildx's attestation manifests, which are not images.
            repo=$(repo_of "$ref")
            while read -r digest platform; do
                bytes=$(docker buildx imagetools inspect --raw "$repo@$digest" | manifest_bytes)
                row "$ref ($platform)" "$bytes"
            done < <(jq -r '.manifests[]
                            | select(.platform.architecture != "unknown")
                            | "\(.digest) \(.platform.os)/\(.platform.architecture)"' <<<"$raw")
        else
            row "$ref" "$(manifest_bytes <<<"$raw")"
        fi
        ;;
    esac
done

printf '\n' >>"$summary"
