#!/bin/sh
# Prefer rustup proxies when an asdf installation shadows the pinned toolchain.
set -eu
if [ -x "$HOME/.cargo/bin/rustup" ]; then
  PATH="$HOME/.cargo/bin:$PATH"
  export PATH
fi
exec cargo "$@"
