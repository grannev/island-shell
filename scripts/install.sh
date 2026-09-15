#!/bin/sh
# Build the release binary and install it to ~/.local/bin (no root needed).
set -e
cd "$(dirname "$0")/.."
cargo build --release
install -Dm755 target/release/island-shell "$HOME/.local/bin/island-shell"
echo "installed $HOME/.local/bin/island-shell"
# the Neovim companion goes on nvim's runtime path (see nvim/README.md)
install -Dm644 nvim/lua/island.lua "$HOME/.local/share/nvim/site/lua/island.lua"
echo "installed $HOME/.local/share/nvim/site/lua/island.lua"
