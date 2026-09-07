# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Builds the browser shell (ui_web/build.sh) and serves ui_web/dist with
# static-web-server (https://static-web-server.net) on its distroless image —
# the page is static, so nothing here needs a shell, a package manager or root.
#
#   docker build -f ui_web/Dockerfile -t grind-web .
#   docker run --rm -p 8080:80 grind-web
#
# The build stage needs the full repo (grind-web depends on grind-core, grind-sheet,
# grind-text), so the context for `docker build` must be the repository root.

FROM docker.io/rust:1-slim-bookworm AS build

# wasm-bindgen's CLI must match the crate version cargo resolves (ui_web/build.sh
# checks this at dev time too); read it from the lockfile so the image can never
# drift from what actually gets compiled.
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
RUN ./ui_web/build.sh release

# static-web-server's own image: distroless, so no shell and no package manager —
# only the binary, CA certs and this build's static files go in.
FROM docker.io/joseluisq/static-web-server:2

COPY --from=build /src/ui_web/dist /public

ENV SERVER_ROOT=/public \
    SERVER_PORT=80

EXPOSE 80
