#!/usr/bin/env bash

LOG_LEVEL=$1
if [[ -z "$LOG_LEVEL" ]]; then
    echo "Usage: $0 <log_level>"
    exit 1
fi



RUST_LOG="info,orchestrator=$1,shared=$1" \
cargo run
