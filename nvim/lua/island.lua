--- island.nvim — the Dynamic Island as Neovim's status line.
---
--- The island keeps a token in ~/.config/island-shell/nvim-token (0600); we
--- quote it in every message, so only an editor running this file ever
--- shows up there. State goes out over the island's control socket and the
--- island calls back into us over our own RPC socket (v:servername), which
--- is how its buttons and buffer list work.
---
--- Drop this on your runtime path and `require("island").setup()`.

local M = {}
local uv = vim.uv or vim.loop

local config = {
  -- the island is the status line, so Neovim does not need one
  statusline = false,
  -- toasts: file saved, LSP errors, LSP / task progress, git branch
  toasts = { saved = true, diagnostics = true, progress = true, git = true },
  -- how often state may be pushed, milliseconds
  throttle = 60,
  -- an LSP job shorter than this never gets a pill (pyright reports one
  -- on every keystroke), milliseconds
  progress_after = 1500,
  -- how many buffers the island's panel lists
  buffers = 6,
}

local state = {
  token = nil,
  socket = nil,
  server = nil,
  pending = false,
  timer = nil,
  last_branch = nil,
  progress = {},
  error_pending = false,
  last_error = nil,
  diagnostic_items = {},
}

local function config_home()
  return vim.env.XDG_CONFIG_HOME or (vim.env.HOME .. "/.config")
end

local function read_token()
  local file = io.open(config_home() .. "/island-shell/nvim-token", "r")
  if not file then
    return nil
  end
  local token = file:read("l")
  file:close()
  return token and vim.trim(token) or nil
end

local function socket_path()
  local runtime = vim.env.XDG_RUNTIME_DIR or "/tmp"
  local display = vim.env.WAYLAND_DISPLAY or "wayland-0"
  return string.format("%s/island-shell-%s.sock", runtime, display)
end

--- Send one line to the island. Fire and forget: the island answers with a
--- line we do not need, and a missing island is not an error worth a
--- message in the editor.
local function send(payload)
  if not state.token then
    return
  end
  payload.protocol = "island.nvim/1"
  payload.token = state.token
  payload.server = state.server
  local ok, json = pcall(vim.json.encode, payload)
  if not ok then
    return
  end
  local pipe = uv.new_pipe(false)
  if not pipe then
    return
  end
  pipe:connect(state.socket, function(err)
    if err then
      pipe:close()
      return
    end
    pipe:write("nvim " .. json .. "\n", function()
      pipe:shutdown(function()
        pipe:close()
      end)
    end)
  end)
end

local function indent_label()
  if vim.bo.expandtab then
    return string.format("%d spaces", vim.bo.shiftwidth > 0 and vim.bo.shiftwidth or vim.bo.tabstop)
  end
  return string.format("%d-wide tabs", vim.bo.tabstop)
end

local function git_info()
  local dict = vim.b.gitsigns_status_dict
  if not dict then
    return "", 0, 0
  end
  return dict.head or "", (dict.added or 0) + (dict.changed or 0), dict.removed or 0
end

local function diagnostics()
  local counts = { errors = 0, warnings = 0, info = 0, hints = 0 }
  local severity = vim.diagnostic.severity
  local by_severity = vim.diagnostic.count(0)
  counts.errors = by_severity[severity.ERROR] or 0
  counts.warnings = by_severity[severity.WARN] or 0
  counts.info = by_severity[severity.INFO] or 0
  counts.hints = by_severity[severity.HINT] or 0
  return counts
end

local function refresh_diagnostics()
  local list = vim.diagnostic.get(0)
  table.sort(list, function(a, b)
    if a.severity ~= b.severity then return (a.severity or 4) < (b.severity or 4) end
    if a.lnum ~= b.lnum then return a.lnum < b.lnum end
    return (a.col or 0) < (b.col or 0)
  end)
  state.diagnostic_items = {}
  for _, d in ipairs(list) do
    table.insert(state.diagnostic_items, {
      buffer = vim.api.nvim_get_current_buf(), line = d.lnum + 1, col = (d.col or 0) + 1,
      severity = d.severity or 4, message = (d.message or ""):gsub("%s+", " "):sub(1, 1000),
    })
    if #state.diagnostic_items == 3 then break end
  end
end

function M.jump(buffer, line, col)
  if not vim.api.nvim_buf_is_valid(buffer) then return false end
  vim.api.nvim_set_current_buf(buffer)
  line = math.min(math.max(1, line), vim.api.nvim_buf_line_count(buffer))
  local text = vim.api.nvim_buf_get_lines(buffer, line - 1, line, false)[1] or ""
  vim.api.nvim_win_set_cursor(0, { line, math.min(math.max(0, col - 1), #text) })
  vim.cmd("normal! zz")
  return true
end

local function search_count()
  if vim.v.hlsearch ~= 1 then
    return nil
  end
  local ok, result = pcall(vim.fn.searchcount, { maxcount = 999, timeout = 40 })
  if not ok or not result or (result.total or 0) == 0 then
    return nil
  end
  return { current = result.current or 0, total = result.total }
end

local function buffers()
  local list = {}
  for _, buf in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_loaded(buf) and vim.bo[buf].buflisted then
      local name = vim.api.nvim_buf_get_name(buf)
      table.insert(list, {
        id = buf,
        name = name ~= "" and vim.fn.fnamemodify(name, ":t") or "[No Name]",
        lang = vim.bo[buf].filetype,
        modified = vim.bo[buf].modified,
        current = buf == vim.api.nvim_get_current_buf(),
      })
    end
    if #list >= config.buffers then
      break
    end
  end
  -- the current buffer leads the list, the rest keep their order
  table.sort(list, function(a, b)
    if a.current ~= b.current then
      return a.current
    end
    return a.id < b.id
  end)
  return list
end

--- Everything the island shows about the editor.
local function collect(focused)
  local line = vim.fn.line(".")
  local lines = vim.fn.line("$")
  local name = vim.api.nvim_buf_get_name(0)
  return {
    event = "state",
    focused = focused ~= false,
    mode = vim.api.nvim_get_mode().mode,
    file = name ~= "" and vim.fn.fnamemodify(name, ":t") or "",
    dir = (function()
      if name == "" then
        return vim.fn.fnamemodify(vim.fn.getcwd(), ":~")
      end
      local dir = vim.fn.fnamemodify(name, ":~:.:h")
      -- a file in the working directory has no path worth showing: say
      -- which directory that is instead
      if dir == "." or dir == "" then
        return vim.fn.fnamemodify(vim.fn.getcwd(), ":~")
      end
      return dir
    end)(),
    lang = vim.bo.filetype,
    modified = vim.bo.modified,
    line = line,
    col = vim.fn.col("."),
    lines = lines,
    percent = lines > 0 and math.floor(line / lines * 100 + 0.5) or 0,
    encoding = vim.bo.fileencoding ~= "" and vim.bo.fileencoding or vim.o.encoding,
    indent = indent_label(),
    diagnostics = diagnostics(),
    diagnostic_items = state.diagnostic_items,
    branch = (git_info()),
    added = select(2, git_info()),
    removed = select(3, git_info()),
    recording = vim.fn.reg_recording(),
    search = search_count(),
    buffers = buffers(),
    cwd = vim.fn.fnamemodify(vim.fn.getcwd(), ":~"),
  }
end

--- State goes out at most every `throttle` ms: a cursor sweeping through a
--- file must not turn into a message per column.
local function push(focused)
  if state.pending then
    state.focused = focused
    return
  end
  state.pending = true
  state.focused = focused
  state.timer:start(config.throttle, 0, function()
    state.pending = false
    vim.schedule(function()
      send(collect(state.focused))
    end)
  end)
end

function M.toast(kind, title, body, progress)
  send({ event = "toast", kind = kind, title = title or "", body = body or "", progress = progress })
end

local function human_size(bytes)
  if bytes < 1024 then
    return string.format("%d B", bytes)
  elseif bytes < 1024 * 1024 then
    return string.format("%.1f kB", bytes / 1024)
  end
  return string.format("%.1f MB", bytes / 1024 / 1024)
end

local function on_write()
  if not config.toasts.saved then
    return
  end
  local name = vim.api.nvim_buf_get_name(0)
  local size = vim.fn.getfsize(name)
  local added, removed = select(2, git_info()), select(3, git_info())
  local parts = { string.format("%d lines", vim.fn.line("$")) }
  if size > 0 then
    table.insert(parts, human_size(size))
  end
  if added + removed > 0 then
    table.insert(parts, string.format("+%d −%d", added, removed))
  end
  M.toast("saved", vim.fn.fnamemodify(name, ":t"), table.concat(parts, "  ·  "))
end

local function first_error()
  local list = vim.diagnostic.get(0, { severity = vim.diagnostic.severity.ERROR })
  if #list == 0 then
    return nil
  end
  local d = list[1]
  local message = vim.split(d.message or "", "\n")[1]
  return string.format("%d:%d", (d.lnum or 0) + 1, (d.col or 0) + 1), message
end

--- The first error, as a pill — but only in normal mode: while you type,
--- the server flags every half-written line, and a pill for each would be
--- noise. Insert-mode changes are held until you come back to normal, and
--- the same error is never announced twice.
local function on_diagnostics()
  if not config.toasts.diagnostics then
    return
  end
  local mode = vim.api.nvim_get_mode().mode
  if mode:sub(1, 1) ~= "n" then
    state.error_pending = true
    return
  end
  state.error_pending = false
  local counts = diagnostics()
  if counts.errors == 0 then
    state.last_error = nil
    return
  end
  local where, message = first_error()
  if not where then
    return
  end
  local name = vim.fn.fnamemodify(vim.api.nvim_buf_get_name(0), ":t")
  local signature = name .. "\0" .. where .. "\0" .. message
  if signature == state.last_error then
    return
  end
  state.last_error = signature
  M.toast("error", string.format("%s  %s", name, where), message)
end

--- Run a shell command and report it in the island: a ring while it runs,
--- the outcome when it is done.  :IslandTask cargo test
function M.task(cmd)
  if type(cmd) == "string" then
    cmd = vim.split(cmd, "%s+")
  end
  local label = table.concat(cmd, " ")
  M.toast("progress", label, "running…", 0.15)
  local start = uv.now()
  vim.system(cmd, { text = true }, function(result)
    local seconds = (uv.now() - start) / 1000
    vim.schedule(function()
      if result.code == 0 then
        M.toast("done", label, string.format("finished in %.1fs", seconds), 1.0)
      else
        local out = vim.split((result.stderr ~= "" and result.stderr or result.stdout) or "", "\n")
        local message = out[1] or ""
        for _, line in ipairs(out) do
          if line:match("error") or line:match("FAILED") or line:match("failed") then
            message = line
            break
          end
        end
        M.toast("error", string.format("%s — exit %d", label, result.code), message)
      end
    end)
  end)
end

function M.setup(opts)
  config = vim.tbl_deep_extend("force", config, opts or {})
  state.token = read_token()
  if not state.token then
    return -- no island on this machine: stay out of the way entirely
  end
  state.socket = socket_path()
  state.server = vim.v.servername
  if state.server == nil or state.server == "" then
    state.server = vim.fn.serverstart()
  end
  state.timer = uv.new_timer()
  -- a heartbeat, so the island can tell a live editor from one that was
  -- killed without a chance to say goodbye
  state.heartbeat = uv.new_timer()
  state.heartbeat:start(10000, 10000, function()
    vim.schedule(function()
      send(collect(state.focused ~= false))
    end)
  end)

  if config.statusline == false then
    -- the island shows the mode, the file, the position, git and the
    -- diagnostics: a status line would only say it again
    vim.o.laststatus = 0
  end

  local group = vim.api.nvim_create_augroup("island", { clear = true })
  local function on(events, callback)
    vim.api.nvim_create_autocmd(events, { group = group, callback = callback })
  end

  on({ "BufEnter", "DiagnosticChanged" }, refresh_diagnostics)
  refresh_diagnostics()
  if config.cmdline ~= false then
    require("island.cmdline").setup({ token = state.token, socket = state.socket, server = state.server, results = config.results })
  end

  on({ "ModeChanged", "BufEnter", "BufWritePost", "TextChanged", "TextChangedI", "DiagnosticChanged", "RecordingEnter", "RecordingLeave", "CmdlineEnter", "CmdlineLeave", "LspAttach", "CursorHold", "WinEnter", "DirChanged" }, function()
    push(true)
  end)
  on({ "CursorMoved", "CursorMovedI" }, function()
    push(true)
  end)
  on("FocusGained", function()
    push(true)
  end)
  on("FocusLost", function()
    push(false)
  end)
  on("VimLeavePre", function()
    send({ event = "gone" })
  end)
  on("BufWritePost", function()
    on_write()
    vim.defer_fn(on_diagnostics, 400)
  end)
  on("DiagnosticChanged", function()
    vim.defer_fn(on_diagnostics, 50)
  end)
  -- back in normal mode: say what the typing left behind
  vim.api.nvim_create_autocmd("ModeChanged", {
    group = group,
    pattern = "*:n*",
    callback = function()
      if state.error_pending then
        vim.defer_fn(on_diagnostics, 150)
      end
    end,
  })

  -- gitsigns tells us the branch through a buffer variable; watch for it
  -- changing to announce a checkout
  on({ "BufEnter", "User" }, function()
    local branch = (git_info())
    if config.toasts.git and branch ~= "" and state.last_branch and branch ~= state.last_branch then
      local added, removed = select(2, git_info()), select(3, git_info())
      M.toast("git", branch, added + removed > 0 and string.format("+%d −%d", added, removed) or "checked out")
    end
    if branch ~= "" then
      state.last_branch = branch
    end
  end)

  -- LSP progress (indexing, formatting): one pill that updates itself
  if config.toasts.progress and vim.fn.has("nvim-0.10") == 1 then
    -- Servers report progress for the smallest things (pyright analyses
    -- the file on every keystroke): a job earns a pill only once it has
    -- run for a while, and the "done" only if the pill was shown at all.
    local jobs = {}
    local function job_key(args)
      local token = args.data.params and args.data.params.token or args.data.result and args.data.result.token
      return tostring(args.data.client_id) .. ":" .. tostring(token)
    end
    on("LspProgress", function(args)
      local client = vim.lsp.get_client_by_id(args.data.client_id)
      local value = args.data.params and args.data.params.value or args.data.result and args.data.result.value
      if not value then
        return
      end
      local name = client and client.name or "LSP"
      local key = job_key(args)
      local job = jobs[key]
      if value.kind == "end" then
        if job then
          if job.timer then
            job.timer:stop()
            job.timer:close()
          end
          if job.shown then
            M.toast("progress", name, value.message or "done", 1.0)
          end
          jobs[key] = nil
        end
        return
      end
      local percentage = value.percentage and value.percentage / 100 or 0.1
      local title = value.title or value.message or "working…"
      if not job then
        job = { shown = false, title = title, percentage = percentage }
        jobs[key] = job
        local timer = uv.new_timer()
        job.timer = timer
        timer:start(config.progress_after, 0, function()
          timer:close()
          job.timer = nil
          vim.schedule(function()
            if jobs[key] == job then
              job.shown = true
              M.toast("progress", name, job.title, job.percentage)
            end
          end)
        end)
        return
      end
      job.title, job.percentage = title, percentage
      if job.shown then
        M.toast("progress", name, title, percentage)
      end
    end)
  end

  -- messages the editor would print in the command line show up in the
  -- island instead
  local notify = vim.notify
  vim.notify = function(message, level, extra)
    local text = type(message) == "string" and vim.split(message, "\n")[1] or tostring(message)
    local kind = "info"
    if level == vim.log.levels.ERROR then
      kind = "error"
    elseif level == vim.log.levels.WARN then
      kind = "warn"
    end
    M.toast(kind, (extra and extra.title) or "Neovim", text)
    return notify(message, level, extra)
  end

  vim.api.nvim_create_user_command("IslandTask", function(args)
    M.task(args.args)
  end, { nargs = "+", complete = "shellcmd", desc = "run a command and report it in the island" })
  vim.api.nvim_create_user_command("IslandPing", function()
    M.toast("info", "Neovim", "the island can hear you")
  end, { desc = "check the island connection" })

  push(true)
end

return M
