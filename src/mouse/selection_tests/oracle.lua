-- Attach a UI so mouse coordinates are interpreted by Neovim's real input loop.
local cases = vim.json.decode([==[__CASES__]==])
local channel = vim.fn.jobstart({ 'nvim', '--clean', '--headless', '--embed', '-n', '-i', 'NONE' }, { rpc = true })
assert(channel > 0, 'start embedded Neovim')
vim.defer_fn(function()
  vim.fn.jobstop(channel)
  io.stderr:write('mouse oracle timed out\n')
  os.exit(1)
end, 10000)
local function req(method, ...)
  return vim.fn.rpcrequest(channel, method, ...)
end
req('nvim_ui_attach', 80, 24, { rgb = true })
local results = {}
for _, case in ipairs(cases) do
  req('nvim_input', '<Esc><Esc>')
  req('nvim_command', 'enew!')
  req('nvim_ui_try_resize', case.width or 80, 24)
  req('nvim_command', 'set mouse=a mousetime=0 nonumber norelativenumber signcolumn=no foldcolumn=0 scrolloff=0 tabstop=4 ' .. (case.wrap and 'wrap' or 'nowrap'))
  local lines = vim.split(case.text:sub(1, -2), '\n', { plain = true })
  req('nvim_buf_set_lines', 0, 0, -1, false, lines)
  req('nvim_win_set_cursor', 0, { 1, 0 })
  req('nvim_command', 'redraw')
  local snapshots = {}
  for _, action in ipairs(case.actions) do
    if action[1] == 'key' then
      req('nvim_input', action[2])
    else
      req('nvim_input_mouse', 'left', action[2], '', 0, action[4], action[3])
    end
    -- A non-fast RPC request runs after queued input; each mouse position
    -- must be processed before sending the next, or Neovim coalesces them.
    snapshots[#snapshots + 1] = req('nvim_exec_lua', [[
      local cursor = vim.api.nvim_win_get_cursor(0)
      local function charcol(line, bytecol)
        local text = vim.api.nvim_buf_get_lines(0, line - 1, line, false)[1]
        return vim.fn.strchars(text:sub(1, bytecol))
      end
      local mode = vim.fn.mode()
      local anchor = vim.NIL
      if mode == 'v' then
        local pos = vim.fn.getpos('v')
        anchor = { pos[2] - 1, charcol(pos[2], pos[3] - 1) }
      end
      return {
        text = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), '\n') .. '\n',
        mode = mode,
        cursor = { cursor[1] - 1, charcol(cursor[1], cursor[2]) },
        anchor = anchor,
      }
    ]], {})
  end
  results[#results + 1] = snapshots
end
pcall(req, 'nvim_command', 'qa!')
io.stdout:write(vim.json.encode(results) .. '\n')
vim.cmd('qa!')
