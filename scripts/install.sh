#!/bin/sh
# Build the release binary and install it to ~/.local/bin (no root needed).
set -e
cd "$(dirname "$0")/.."
cargo build --release
install -Dm755 target/release/island-shell "$HOME/.local/bin/island-shell"
echo "installed $HOME/.local/bin/island-shell"
