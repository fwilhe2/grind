#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# A reproducible local model server for the formula assistant (doc/formula-assistant.md, A1):
# llama.cpp's server from a container image pinned by digest (ci/llama-server-image), and a model
# file pinned by repository revision and SHA-256 (assist/models.tsv). Grind itself downloads
# nothing and opens no socket but loopback; this script is the one place a model is fetched.
#
#   scripts/llama-server.sh fetch [MODEL]    # download and verify; idempotent
#   scripts/llama-server.sh serve [MODEL]    # fetch, then serve on 127.0.0.1:8080 in the background
#   scripts/llama-server.sh status           # is it up, and which model
#   scripts/llama-server.sh logs | stop
#   scripts/llama-server.sh models           # what assist/models.tsv offers
#
# MODEL is a name from assist/models.tsv; the default is the recommended one, smollm3-3b.
#
# Environment: GRIND_MODELS (where model files live; default $XDG_DATA_HOME/grind/models),
# GRIND_LLAMA_PORT (8080), GRIND_LLAMA_CTX (8192 tokens), GRIND_LLAMA_THREADS (physical cores),
# GRIND_LLAMA_ARGS (more server flags, e.g. "-fa on -ctk q8_0 -ctv q8_0" to halve a 7B model's
# attention cache on a machine short of memory), CONTAINER (docker, else podman).
#
# The server answers the same request with the same bytes: one slot (`-np 1`), so a request never
# shares a batch with another, and the benchmark asks with `temperature: 0` and a fixed seed.
set -euo pipefail
cd "$(dirname "$0")/.."

models_dir=${GRIND_MODELS:-${XDG_DATA_HOME:-$HOME/.local/share}/grind/models}
port=${GRIND_LLAMA_PORT:-8080}
ctx=${GRIND_LLAMA_CTX:-8192}
name=grind-llama
image=$(cat ci/llama-server-image)
engine=${CONTAINER:-$(command -v docker >/dev/null && echo docker || echo podman)}

# Physical cores, not hardware threads: llama.cpp's token generation is memory-bound and gets
# slower, not faster, with a hyper-thread per core.
threads=${GRIND_LLAMA_THREADS:-$(lscpu -p=core 2>/dev/null | grep -v '^#' | sort -u | wc -l)}
[ "$threads" -gt 0 ] || threads=4

row() {
    local model=${1:-smollm3-3b}
    awk -F'\t' -v m="$model" '!/^#/ && $1 == m' assist/models.tsv | grep . ||
        { echo "no model '$model' in assist/models.tsv; try: $0 models" >&2; exit 1; }
}

fetch() {
    local file url sha
    IFS=$'\t' read -r _ file url sha _ _ <<<"$(row "${1:-}")"
    mkdir -p "$models_dir"
    local path=$models_dir/$file
    if [ -f "$path" ] && echo "$sha  $path" | sha256sum --check --status; then
        echo "$path" ; return
    fi
    echo "downloading $file into $models_dir" >&2
    curl -fL --retry 3 -C - -o "$path.part" "$url"
    echo "$sha  $path.part" | sha256sum --check --status ||
        { echo "$file: SHA-256 does not match assist/models.tsv; removed" >&2; rm -f "$path.part"; exit 1; }
    mv "$path.part" "$path"
    echo "$path"
}

case ${1:-} in
    fetch) fetch "${2:-}" ;;
    serve)
        path=$(fetch "${2:-}")
        "$engine" rm -f "$name" >/dev/null 2>&1 || true
        # Published on loopback only: a model server reachable from the network would make the
        # document text in every prompt reachable from it too.
        "$engine" run -d --name "$name" \
            -p "127.0.0.1:$port:8080" \
            -v "$models_dir:/models:ro,Z" \
            "$image" \
            -m "/models/$(basename "$path")" \
            --host 0.0.0.0 --port 8080 \
            -c "$ctx" -np 1 --threads "$threads" --seed 0 --jinja \
            ${GRIND_LLAMA_ARGS:-} >/dev/null
        echo "serving $(basename "$path") on http://127.0.0.1:$port with $threads threads; waiting for it to load" >&2
        for _ in $(seq 1 120); do
            curl -sf "http://127.0.0.1:$port/health" >/dev/null && { echo "ready" >&2; exit 0; }
            sleep 1
        done
        echo "not ready after two minutes; see: $0 logs" >&2
        exit 1
        ;;
    status)
        curl -sf "http://127.0.0.1:$port/health" && echo
        curl -sf "http://127.0.0.1:$port/v1/models" | grep -o '"id":"[^"]*"' || { echo "not running" >&2; exit 1; }
        ;;
    logs) exec "$engine" logs -f "$name" ;;
    stop) "$engine" rm -f "$name" ;;
    models) grep -v '^#' assist/models.tsv | cut -f1,2,5,6 | column -t ;;
    *) awk 'NR >= 7 && !/^#/ { exit } NR >= 7 { sub(/^# ?/, ""); print }' "$0"; exit 2 ;;
esac
