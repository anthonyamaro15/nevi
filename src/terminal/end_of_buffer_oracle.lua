-- Attach a UI so screenstring() can read the rendered window.
local channel = vim.fn.jobstart({ 'nvim', '--clean', '--headless', '--embed', '-n', '-i', 'NONE' }, { rpc = true })
assert(channel > 0, 'start embedded Neovim')
vim.defer_fn(function()
  vim.fn.jobstop(channel)
  io.stderr:write('end-of-buffer oracle timed out\n')
  os.exit(1)
end, 10000)

vim.fn.rpcrequest(channel, 'nvim_ui_attach', 40, 10, { rgb = true })
local rows = vim.fn.rpcrequest(channel, 'nvim_exec_lua', [[
  vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'a' })
  vim.cmd('set nu scl=yes')
  local snapshots = {}
  for _, wrap in ipairs({ 'nowrap', 'wrap' }) do
    vim.cmd('set ' .. wrap)
    vim.cmd('redraw!')
    local rows = {}
    for row = 1, 3 do
      local cells = {}
      for column = 1, 40 do
        cells[column] = vim.fn.screenstring(row, column)
      end
      rows[row] = table.concat(cells):gsub(' +$', '')
    end
    snapshots[#snapshots + 1] = rows
  end
  return snapshots
]], {})
pcall(vim.fn.rpcrequest, channel, 'nvim_command', 'qa!')
io.stdout:write(vim.json.encode(rows) .. '\n')
