#!/bin/sh
# Build the release binary and install it to ~/.local/bin (no root needed).
set -e
cd "$(dirname "$0")/.."
cargo build --release
install -Dm755 target/release/island-shell "$HOME/.local/bin/island-shell"
install -Dm755 scripts/island-screenshot.sh "$HOME/.local/bin/island-screenshot"
install -Dm644 shell/island.bash "$HOME/.local/share/island-shell/island.bash"
install -Dm644 assets/waybar/timer.svg "$HOME/.local/share/island-shell/icons/timer.svg"
install -Dm644 assets/waybar/bluetooth.svg "$HOME/.local/share/island-shell/icons/bluetooth.svg"
echo "installed $HOME/.local/bin/island-shell"
# the user service (started by the sway config, see README)
install -Dm644 systemd/island-shell.service "$HOME/.config/systemd/user/island-shell.service"
systemctl --user daemon-reload 2>/dev/null || true
# the Neovim companion goes on nvim's runtime path (see nvim/README.md)
install -Dm644 nvim/lua/island.lua "$HOME/.local/share/nvim/site/lua/island.lua"
install -Dm644 nvim/lua/island/cmdline.lua "$HOME/.local/share/nvim/site/lua/island/cmdline.lua"
install -Dm644 nvim/lua/island/results.lua "$HOME/.local/share/nvim/site/lua/island/results.lua"
echo "installed $HOME/.local/share/nvim/site/lua/island.lua"
# a running island keeps the old binary until it restarts
if systemctl --user is-active --quiet island-shell 2>/dev/null; then
    systemctl --user restart island-shell
    echo "restarted island-shell.service"
fi
