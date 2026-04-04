#!/bin/bash

# Option --compile: rebuild libjs and test262-runner before running
if [ "$1" = "--compile" ]; then
  echo "=== Compiling test262-runner ==="
  (cd tools/test262-runner && RUSTFLAGS="-Awarnings" cargo +stable build --release 2>&1 | grep -E '^(   Compiling|    Finished|error)') || exit 1
  echo "=== Compilation done ==="
  shift
fi

for start in 1 1001 2001 3001 4001 5001 6001 7001 8001 9001 10001 11001 12001 13001 14001 15001 16001 17001 18001 19001 20001 21001 22001 23001 24001 25001 26001; do
  echo "=== Block start=$start ==="
  tools/test262-runner/target/x86_64-unknown-linux-gnu/release/test262-runner libs/libjs_tests/test262 --limit 1000 --start $start --verbose --timeout 5 2>&1 >/tmp/test262.$start.txt
  echo -n "wait ....."
  sleep 1
  echo " ok"
done
