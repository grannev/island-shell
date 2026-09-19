# island.nvim

The island doubles as Neovim's status line: the mode, the file, the
position, git and the diagnostics live at the top of the screen, saving and
LSP progress arrive as pills, and resting the pointer on the island opens a
panel with the buffer list and Save / Format / Close.

## Install

This directory is a plugin (`lua/island.lua`): point lazy.nvim at it, or
let `scripts/install.sh` copy the module to
`~/.local/share/nvim/site/lua/island.lua` and use the runtime path.
[iVim](../../ivim) — the Neovim configuration built around the island —
does the former:

```lua
-- lua/plugins/island.lua
return {
  {
    "nvim-lualine/lualine.nvim",
    enabled = false, -- the island is the status line now
  },
  {
    dir = vim.fn.expand("~/it/coding/island-shell/nvim"),
    name = "island.nvim",
    lazy = false,
    priority = 100,
    config = function()
      require("island").setup()
    end,
  },
}
```

Without the island running (or on another machine) `setup()` finds no token
and returns at once, leaving the editor exactly as it was.

## What it sends

State — mode, file and directory, language, modified flag, cursor position
and percentage, encoding and indent, diagnostics counts, git branch and
diff, the register a macro is recording into, the search count and the
buffer list — at most every 60 ms, and only over the island's own socket.

Toasts — `saved` on write (lines, size, +n −m), `error` for the first LSP
error (held while you type, announced when you return to normal mode, and
never twice for the same error), `progress` for LSP jobs and `:IslandTask`, `git` when the branch
changes. `vim.notify` goes to the island too.

## Commands

    :IslandTask cargo test    run a command, show a ring while it runs
    :IslandPing               check that the island can hear you

## Options

```lua
require("island").setup({
  statusline = false,                 -- true keeps laststatus alone
  toasts = { saved = true, diagnostics = true, progress = true, git = true },
  throttle = 60,                      -- ms between state pushes
  progress_after = 1500,              -- an LSP job shorter than this gets no pill
  buffers = 6,                        -- rows the island's panel may list
})
```

## Security

The island writes `~/.config/island-shell/nvim-token` (0600) on first run
and ignores any message that does not quote it, so a Neovim without this
plugin — or another user's process — never appears in the island.

## Command line and diagnostics

The iVim configuration currently disables island command input and output.
The implementation below is archived on the
`archive/island-command-ui-2026-09-19` branch for later work.

The companion presents `:`, `/` and `?` in an 84-pixel island sheet. Neovim
keeps keyboard focus, command history, execution and search. The island must
acknowledge that it is visible before Neovim hides its native command line.
If the island is hidden, busy with another sheet, outdated or unreachable,
the native command line returns automatically (within roughly 430 ms).
The connection is checked only while input is active; abandoned input in the
island expires after 800 ms. Password and complex nested prompts stay native.
Use `:IslandCommandLine off` for the current session, or `cmdline = false`
in `setup()` to keep native input permanently.

Click the diagnostic counters in the editor panel to switch its three rows
from buffers to problems in the current file, ordered by severity and line.
Click a problem to jump to its exact buffer, line and column. Click `‹ Buffers`
to return. Switching files resets the panel to buffers.

Short results (up to 240 bytes / two lines) from simple `:echo`, `:echomsg`,
`:echoerr`, `:pwd`, `:file` and option queries appear as island notifications.
Command parse errors are shown there too. Evaluated function calls, chained
commands, plugin commands, interactive prompts and long output stay native;
`:messages` keeps its native history view. Failed or unacknowledged delivery
replays the result in Neovim. Set `results = false` in `setup()` to disable
result forwarding independently of island input.

Regression checks: `cargo test --offline --bin island-shell`, `python3 tests/nvim_input.py`
`python3 tests/nvim_results.py` and `python3 tests/nvim_companion.py`. The Python checks need pynvim and
a local Unix socket; the companion check also covers diagnostics and jumps.
