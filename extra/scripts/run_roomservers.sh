#!/usr/bin/env bash

LOG_LEVEL=$1
if [[ -z "$LOG_LEVEL" ]]; then
    echo "Usage: $0 <log_level> <number_of_instances>"
    exit 1
fi

NUM_INSTANCES=$2
if [[ -z "$NUM_INSTANCES" ]]; then
    echo "Usage: $0 <log_level> <number_of_instances>"
    exit 1
fi

PIDS=()

cleanup() {
    echo "Killing all spawned processes..."
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null
    done
    exit
}

trap cleanup SIGINT

cargo build --package roomserver

if [ "$?" -ne "0" ]; then
    exit -1;
fi

for ((i=1; i<=NUM_INSTANCES; i++)); do
    RANDOM_NUMBER=$(( RANDOM % 101 ))
    echo "Starting instance $i with argument $RANDOM_NUMBER"
    
    RUST_LOG="info,roomserver=$LOG_LEVEL,shared=trace" \
    ./target/debug/roomserver "$RANDOM_NUMBER" &
    
    PIDS+=($!)
done

wait
