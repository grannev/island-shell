-- Neovim owns editing and focus; the island only presents acknowledged input.
local M = {}

function M.setup(connection)
    local uv = vim.uv
    local ns = vim.api.nvim_create_namespace("island.command")
    local levels, enabled, pending, generation = {}, false, false, 0
    local timer = uv.new_timer()
    local stopped = false
    local function active()
        local top, command = 0, nil
        for level, value in pairs(levels) do
            if level > top then top, command = level, value end
        end
        return command
    end
    local ui_handler, results
    local disabled = false
    local function visibility(value)
        if enabled == value then return end
        enabled = value
        if value then
            vim.ui_attach(ns, { ext_cmdline = true, ext_messages = connection.results ~= false, set_cmdheight = false }, ui_handler)
        else
            vim.ui_detach(ns)
        end
        if vim.fn.getcmdtype() ~= "" then
            vim.fn.setcmdline(vim.fn.getcmdline(), vim.fn.getcmdpos())
        end
        vim.cmd("redraw!")
    end
    local request
    request = function()
        if pending or stopped or disabled then return end
        pending = true
        local current = generation
        local pipe, timeout = uv.new_pipe(false), uv.new_timer()
        local finished, reply = false, ""
        local function finish(ok)
            if finished then return end
            finished, pending = true, false
            timeout:stop()
            timeout:close()
            if not pipe:is_closing() then pipe:close() end
            vim.schedule(function()
                if stopped then return end
                -- Never let a late acknowledgement hide a newer prompt.
                if not ok then
                    visibility(false)
                elseif current == generation then
                    visibility(active() ~= nil or connection.results ~= false)
                end
                if current ~= generation then request() end
            end)
        end
        timeout:start(180, 0, function() finish(false) end)
        pipe:connect(connection.socket, function(err)
            if finished then return end
            if err then finish(false); return end
            local payload = {
                protocol = "island.nvim/1", event = "cmdline", token = connection.token,
                server = connection.server, command = active() or vim.NIL,
            }
            pipe:read_start(function(read_error, data)
                if finished then return end
                if read_error or not data then finish(false); return end
                reply = reply .. data
                if reply:find("\n", 1, true) then finish(reply:match("^ok: cmdline") ~= nil) end
            end)
            pipe:write("nvim " .. vim.json.encode(payload) .. "\n", function(write_error)
                if write_error then finish(false) end
            end)
        end)
    end
    local function changed()
        generation = generation + 1
        vim.schedule(request)
        timer:stop()
        if active() then
            timer:start(250, 250, vim.schedule_wrap(request))
        elseif connection.results == false then
            visibility(false)
        end
    end
    results = require("island.results").new(connection, function() visibility(false) end)
    local function update()
        local kind = vim.fn.getcmdtype()
        local next_command
        if kind == ":" or kind == "/" or kind == "?" then
            next_command = { text = vim.fn.getcmdline(), pos = vim.fn.getcmdpos() - 1, prefix = kind }
        end
        -- Prompt content from UI events is already masked for inputsecret().
        if kind ~= ":" and kind ~= "/" and kind ~= "?" and kind ~= "" then return end
        if not vim.deep_equal(levels[1], next_command) then
            levels = { next_command }
            changed()
        end
    end
    ui_handler = function(event, ...)
        if results.handle(event, ...) then return true end
        if event == "cmdline_show" then
            local content, pos, first, prompt, _, level = ...
            local chunks = {}
            for _, chunk in ipairs(content) do chunks[#chunks + 1] = chunk[2] end
            local command = { text = table.concat(chunks), pos = pos, prefix = first .. prompt }
            if not vim.deep_equal(levels[level], command) then
                levels[level] = command
                vim.schedule(changed)
            end
        elseif event == "cmdline_hide" then
            local level = ...
            if levels[level] then levels[level] = nil; vim.schedule(changed) end
        elseif event == "cmdline_pos" then
            local pos, level = ...
            if levels[level] and levels[level].pos ~= pos then
                levels[level].pos = pos
                vim.schedule(changed)
            end
        end
        return true
    end
    local group = vim.api.nvim_create_augroup("island.command", { clear = true })
    vim.api.nvim_create_autocmd({ "CmdlineEnter", "CmdlineChanged" }, { group = group, callback = update })
    vim.api.nvim_create_autocmd("CmdlineLeave", { group = group, callback = function(args)
        local command = active()
        local capture = enabled and command and command.prefix == ":" and not vim.v.event.abort
        levels = {}
        if capture and connection.results ~= false then results.begin(command.text) end
        changed()
    end })
    vim.api.nvim_create_user_command("IslandCommandLine", function(args)
        if args.args == "off" then
            levels = {}
            changed()
            disabled = true
            timer:stop()
            visibility(false)
            vim.api.nvim_clear_autocmds({ group = group })
            vim.cmd("redraw!")
        end
    end, { nargs = 1, complete = function() return { "off" } end })
    vim.api.nvim_create_user_command("IslandOutput", function() results.show() end, {})
    vim.api.nvim_create_autocmd("VimLeavePre", { once = true, callback = function()
        stopped = true
        timer:stop()
        timer:close()
    end })
end

return M
