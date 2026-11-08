#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# The browser shell and `grind serve` under rootless Podman, on this computer.
#
#   ui_web/deploy/podman.sh build              # the image from this checkout, as grind-web:local
#   ui_web/deploy/podman.sh run [DIR]          # serve DIR at http://127.0.0.1:8080/ (Ctrl+C stops)
#   ui_web/deploy/podman.sh kube [DIR]         # kubernetes.yaml's pod, proxy and all, at :8081
#   ui_web/deploy/podman.sh kube-down
#
# DIR defaults to ./documents and is made if missing. `run` and `kube` use grind-web:local when
# it has been built, and the published ghcr.io/fwilhe2/grind-web:latest otherwise; GRIND_IMAGE
# names another, and GRIND_PORT another port for `run` (8080). Files the server writes are yours:
# the container runs as your own uid.

set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
published="ghcr.io/fwilhe2/grind-web:latest"

image() {
    if [ -n "${GRIND_IMAGE:-}" ]; then
        echo "$GRIND_IMAGE"
    elif podman image exists grind-web:local; then
        echo grind-web:local
    else
        echo "$published"
    fi
}

documents() {
    local dir="${1:-documents}"
    mkdir -p "$dir"
    (cd "$dir" && pwd)
}

case "${1:-}" in
    build)
        exec podman build -f "$root/ui_web/Dockerfile" -t grind-web:local "$root"
        ;;
    run)
        dir="$(documents "${2:-}")"
        port="${GRIND_PORT:-8080}"
        echo "grind-web: $dir at http://127.0.0.1:$port/"
        # The host's network, because grind listens on 127.0.0.1 only; keep-id, so the
        # container's user is you and not a subordinate uid that cannot write $dir.
        exec podman run --rm --init --name grind-web --network host \
            --userns=keep-id --user "$(id -u):$(id -g)" \
            --read-only --cap-drop ALL --security-opt no-new-privileges \
            -v "$dir:/documents:z" "$(image)" serve /documents --port "$port"
        ;;
    kube)
        dir="$(documents "${2:-}")"
        # kubernetes.yaml as it stands, with three local changes: the image, your uid for the
        # pod's, and the documents as this directory rather than a claim on a cluster's storage.
        sed -e "s|image: $published|image: $(image)|" \
            -e "s|runAsUser: 65532|runAsUser: $(id -u)|" \
            -e "s|runAsGroup: 65532|runAsGroup: $(id -g)|" \
            -e "s|fsGroup: 65532|fsGroup: $(id -g)|" \
            -e "s|imagePullPolicy: Always|imagePullPolicy: IfNotPresent|" \
            "$root/ui_web/deploy/kubernetes.yaml" \
            | awk -v dir="$dir" '
                /persistentVolumeClaim:/ { print "          hostPath:"; skip = 1; next }
                skip && /claimName:/ { print "            path: " dir; print "            type: Directory"; skip = 0; next }
                { print }' \
            | podman kube play --userns=keep-id --publish 8081:8081 -
        echo "grind-web: $dir at http://localhost:8081/ (through the pod's proxy)"
        ;;
    kube-down)
        # The volumes kube play made for the claim and the ConfigMap go too, the second by
        # name since --force leaves it. The documents are the directory, which stays.
        podman kube down --force "$root/ui_web/deploy/kubernetes.yaml"
        podman volume rm --force grind-web-proxy >/dev/null
        ;;
    *)
        sed -n '8,16p' "$0" | sed 's/^# \{0,1\}//' >&2
        exit 2
        ;;
esac
