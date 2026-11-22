#!/bin/sh
# Prefer rustup proxies when an asdf installation shadows the pinned toolchain.
set -eu
if [ -x "$HOME/.cargo/bin/rustup" ]; then
  PATH="$HOME/.cargo/bin:$PATH"
  export PATH
fi
# Cargo starts the runner separately for each test executable; no environment
# mutation occurs after libtest starts its threads.
if [ "${1-}" = test ]; then
  script_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
  runner_config=$(python3 -B -c 'import json,sys; print("target.\"cfg(all())\".runner=" + json.dumps(["python3", "-B", sys.argv[1]]))' "$script_root/tests/support/run_test.py")
  exec cargo --config "$runner_config" "$@"
fi
exec cargo "$@"
