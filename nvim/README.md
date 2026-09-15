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
error, `progress` for LSP jobs and `:IslandTask`, `git` when the branch
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
