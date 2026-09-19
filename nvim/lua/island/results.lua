-- Messages share the command UI subscription, so streamed and asynchronous
-- output is captured without repeatedly attaching/detaching Neovim's UI.
local M = {}
local uv = vim.uv
local current

function M.new(connection, fallback)
    local messages, title, serial = {}, "Command output", 0
    local pending, scheduled, dirty, last = false, false, false, nil
    local queue = {}
    local pump
    local function snapshot()
        local lines, failed = {}, false
        for _, message in ipairs(messages) do
            lines[#lines + 1] = message.text
            failed = failed or message.error
        end
        return {
            protocol = "island.nvim/1", event = "result", token = connection.token,
            server = connection.server, id = tostring(serial), append = false,
            title = title, body = table.concat(lines, "\n"), kind = failed and "error" or "info",
        }
    end
    local function enqueue()
        if not dirty then return end
        dirty = false
        local payload = snapshot()
        if payload.body == "" then return end
        last = payload
        if queue[#queue] and queue[#queue].id == payload.id then queue[#queue] = payload
        else queue[#queue + 1] = payload end
        pump()
    end
    local function changed()
        dirty = true
        if scheduled then return end
        scheduled = true
        vim.defer_fn(function() scheduled = false; enqueue() end, 30)
    end
    pump = function()
        if pending or #queue == 0 then return end
        local payload = table.remove(queue, 1)
        pending = true
        local pipe, timer = uv.new_pipe(false), uv.new_timer()
        local ended, reply = false, ""
        local function finish(ok)
            if ended then return end
            ended = true
            timer:stop(); timer:close()
            if not pipe:is_closing() then pipe:close() end
            vim.schedule(function()
                pending = false
                if not ok then
                    fallback()
                    table.insert(queue, 1, payload)
                    if dirty then queue[#queue + 1] = snapshot(); dirty = false end
                    local latest, order = {}, {}
                    for _, item in ipairs(queue) do
                        if not latest[item.id] then order[#order + 1] = item.id end
                        latest[item.id] = item
                    end
                    queue = {}
                    for _, id in ipairs(order) do
                        local item = latest[id]
                        vim.api.nvim_echo({ { item.body, item.kind == "error" and "ErrorMsg" or "Normal" } }, false, {})
                    end
                else
                    pump()
                end
            end)
        end
        timer:start(500, 0, function() finish(false) end)
        pipe:connect(connection.socket, function(err)
            if ended then return end
            if err then finish(false); return end
            pipe:read_start(function(read_error, data)
                if ended then return end
                if read_error or not data then finish(false); return end
                reply = reply .. data
                if reply:find("\n", 1, true) then finish(reply:match("^ok: result") ~= nil) end
            end)
            pipe:write("nvim " .. vim.json.encode(payload) .. "\n", function(err_write)
                if err_write then finish(false) end
            end)
        end)
    end
    local object = {}
    function object.begin(command)
        enqueue()
        serial = serial + 1
        title = ":" .. command
        messages = {}
    end
    function object.handle(event, ...)
        if event == "msg_show" then
            local kind, content, replace, _, append, id = ...
            local chunks = {}
            for _, chunk in ipairs(content) do chunks[#chunks + 1] = chunk[2] end
            local message = { text = table.concat(chunks), id = id,
                error = kind == "emsg" or kind == "echoerr" or kind == "lua_error" or kind == "rpc_error" }
            if kind == "empty" then return true end
            if append and #messages > 0 then
                local previous = messages[#messages]
                previous.text = previous.text .. message.text
                previous.error = previous.error or message.error
            else
                local index
                if id ~= nil then
                    for i, item in ipairs(messages) do if item.id == id then index = i; break end end
                end
                if not index and replace and #messages > 0 then index = #messages end
                messages[index or (#messages + 1)] = message
            end
            changed()
            return true
        elseif event == "msg_history_show" then
            local entries = ...
            messages = {}
            for _, entry in ipairs(entries) do
                local chunks = {}
                for _, chunk in ipairs(entry[2]) do chunks[#chunks + 1] = chunk[2] end
                messages[#messages + 1] = { text = table.concat(chunks), error = entry[1] == "emsg" }
            end
            changed()
            return true
        elseif event == "cmdline_block_show" or event == "cmdline_block_append" then
            local content = ...
            if event == "cmdline_block_append" then content = { content } end
            for _, line in ipairs(content) do
                local chunks = {}
                for _, chunk in ipairs(line) do chunks[#chunks + 1] = chunk[2] end
                messages[#messages + 1] = { text = table.concat(chunks) }
            end
            changed()
            return true
        elseif event:match("^msg_") then
            return true
        end
        return false
    end
    function object.show()
        if not last then return end
        serial = serial + 1
        title = last.title
        messages = { { text = last.body, error = last.kind == "error" } }
        changed()
    end
    function object.publish(command, body, error)
        object.begin(command)
        messages = { { text = body, error = error } }
        changed()
    end
    current = object
    return object
end

function M.show() if current then current.show() end end
function M.publish(command, body, error) if current then current.publish(command, body, error) end end
return M
