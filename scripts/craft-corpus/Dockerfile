# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# The Craft corpus in a container: grind built from this checkout, the pinned Craft CLIs, and
# the generator. From the repository root:
#
#   docker build -f scripts/craft-corpus/Dockerfile -t grind-craft-corpus .
#   docker run --rm -v "$PWD/target/craft-corpus:/out" grind-craft-corpus
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p grind-cli

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl python3 \
    && rm -rf /var/lib/apt/lists/*
COPY scripts/craft-corpus /craft
RUN /craft/install.sh && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/grind /usr/local/bin/grind
ENV GRIND_BIN=/usr/local/bin/grind WORDCRAFT_REGION=en-US
ENTRYPOINT ["/craft/run.sh", "/out"]
