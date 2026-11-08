# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# The browser shell and `grind serve` behind it (doc/server.md), on a distroless image: the
# page, the `grind` binary, and nothing else — no shell, no package manager, no root.
#
#   docker build -f ui_web/Dockerfile -t grind-web .
#   docker run --rm --init --network host -v "$PWD/documents:/documents:z" grind-web
#
# `grind serve` listens on 127.0.0.1 only (doc/local-only.md), so a published port (`-p`)
# cannot reach it: the container shares the host's network, and the page is at
# http://127.0.0.1:8080/ on that host. ui_web/deploy/kubernetes.yaml reaches it through a proxy
# in the same pod instead. `--init` because grind as PID 1 does not stop on SIGTERM.
#
# There is no git in the image, so the page offers no history: run `grind serve` itself over a
# git repository for that.
#
# The context is the repository root: the page and the binary are built from one tree, which
# is what keeps them the same build.

FROM docker.io/rust:1-bookworm AS build

RUN rustup target add wasm32-unknown-unknown

WORKDIR /src

# Cargo.lock alone decides which wasm-bindgen-cli this needs, so it arrives on its
# own: compiling the tool then lands in a layer that only a dependency change
# invalidates, rather than one that any edit to any source file rebuilds from
# scratch. That layer is the expensive one, and it is what the workflow's
# `cache-from: type=gha` is there to hit.
COPY Cargo.lock .
RUN version="$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock \
        | sed -n 's/^version = "\(.*\)"/\1/p' | head -n1)" \
    && cargo install wasm-bindgen-cli --version "$version" --locked

COPY . .
RUN ./ui_web/build.sh release && cargo build --release --locked -p grind-cli

# Bookworm, as the build stage, so the binary asks for no newer glibc than this has.
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=build /src/target/release/grind /usr/bin/grind
# Where `grind serve` looks for the page when it is given no --page.
COPY --from=build /src/ui_web/dist /usr/share/grind/web

VOLUME /documents
ENTRYPOINT ["/usr/bin/grind"]
CMD ["serve", "/documents", "--port", "8080"]
