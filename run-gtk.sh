#!/bin/bash

cargo build
SHEET=target/debug/sheet examples/sample.sh /tmp/demo
cargo run -p sheet-gtk -- /tmp/demo/sample.fods

