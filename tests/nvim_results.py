import pynvim, socket, threading, json, time, os, tempfile
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
fixture = tempfile.TemporaryDirectory(prefix="island-input-")
path=str(Path(fixture.name) / 'island.sock')
try: os.unlink(path)
except FileNotFoundError: pass
listener=socket.socket(socket.AF_UNIX); listener.bind(path); listener.listen()
state={'reply':b'ok: cmdline\n','last':None,'results':[]}
def serve():
 while True:
  conn,_=listener.accept()
  with conn:
   data=b''
   while not data.endswith(b'\n'):
    part=conn.recv(65536)
    if not part: break
    data+=part
   payload=json.loads(data[5:])
   state['last']=payload
   if payload['event']=='result': state['results'].append(payload)
   if state['reply']:
    conn.sendall(b'ok: result\n' if payload['event']=='result' and state['reply']==b'ok: cmdline\n' else state['reply'])
   else: time.sleep(.3)
threading.Thread(target=serve,daemon=True).start()
n=pynvim.attach('child',argv=['nvim','--clean','--embed','-n'])
n.ui_attach(100,35,rgb=True,ext_linegrid=True)
n.exec_lua('''
local runtime, socket = ...
vim.opt.rtp:prepend(runtime)
local attach, detach = vim.ui_attach, vim.ui_detach
_G.attached = false
vim.ui_detach = function(ns) _G.attached = false; return detach(ns) end
_G.events = {}
vim.ui_attach = function(ns,opts,callback)
 _G.attached = true
 return attach(ns,opts,function(event,...)
   local result = callback(event,...)
   table.insert(_G.events,{event=event,handled=not not result})
   return result
 end)
end
require('island.cmdline').setup({socket=socket,token='test',server='test'})
''', str(ROOT / 'nvim'), path)
def wait(ms=400):
 until=time.monotonic()+ms/1000
 while time.monotonic()<until:
  n.request('nvim_get_mode'); time.sleep(.02)
def event(): return n.exec_lua('return _G.events[#_G.events]')
grid={}
def screen():
 for msg in list(n._session._pending_messages):
  if msg[0] != 'notification' or msg[1] != 'redraw': continue
  for group in msg[2]:
   if group[0]=='grid_line':
    for g,row,col,cells,*rest in group[1:]:
     for cell in cells:
      for _ in range(cell[2] if len(cell)>2 else 1):
       grid[(row,col)]=cell[0];col+=1
   elif group[0]=='grid_scroll':
    for g,top,bottom,left,right,rows,cols in group[1:]:
     before=grid.copy()
     for row in range(top,bottom):
      for col in range(left,right):
       grid[(row,col)]=before.get((row+rows,col+cols),' ') if top<=row+rows<bottom and left<=col+cols<right else ' '
   elif group[0]=='grid_clear': grid.clear()
 n._session._pending_messages.clear()
 return [''.join(grid.get((row,col),' ') for col in range(100)) for row in range(35)]

def command(text):
 print('CHECK',text[:65],flush=True)
 n.input(':'+text); wait()
 n.input('<CR>'); wait()
try:
 command('echo "Привет"')
 assert state['results'][-1]['body']=='Привет',state
 assert not any('Привет' in row for row in screen())
 command('IslandNoSuchCommand')
 assert state['results'][-1]['kind']=='error',state['results']
 n.input(':echo "native_result"'); wait()
 state['reply']=b'fallback\n'
 n.input('<CR>'); wait()
 assert any('native_result' in row for row in screen()),screen()
 state['reply']=b'ok: cmdline\n'
 n.input(':echo "timeout_result"'); wait()
 state['reply']=None
 n.input('<CR>'); wait(800)
 assert any('timeout_result' in row for row in screen()),screen()
 state['reply']=b'ok: cmdline\n'
 command('pwd')
 assert state['results'][-1]['body']==os.getcwd(),state['results']
 command('set tabstop?')
 assert 'tabstop=' in state['results'][-1]['body'],state['results']
 command('echo "' + 'x'*260 + '"')
 assert state['results'][-1]['body']=='x'*260,state['results'][-1]
 command('echo repeat("Привет 世界 ", 2000)')
 assert state['results'][-1]['body']=='Привет 世界 '*2000
 command('lua print("lua_output")')
 assert 'lua_output' in state['results'][-1]['body'],state['results'][-1]
 n.exec_lua("vim.api.nvim_create_user_command('TestOutput', function() print('plugin_output') end, {})")
 command('TestOutput')
 assert 'plugin_output' in state['results'][-1]['body'],state['results'][-1]
 command('echomsg "history_entry"')
 command('messages')
 assert 'history_entry' in state['results'][-1]['body'],state['results'][-1]
 command('lua vim.defer_fn(function() print("async_output") end, 100)')
 assert 'async_output' in state['results'][-1]['body'],state['results'][-1]
 command('!printf "shell_output\\n"')
 assert any('shell_output' in r['body'] for r in state['results']),state['results'][-1]
 n.input('<CR><Esc>'); wait()
 command('echo confirm("Continue?", "&Yes\\n&No", 2)')
 assert 'Continue?' in state['results'][-1]['body'],state['results'][-1]
 assert '[N]o' in state['last']['command']['prefix'],state['last']
 n.input('n<CR>'); wait()
 assert n.request('nvim_get_mode')['mode']=='n',n.request('nvim_get_mode')
 n.exec_lua("vim.api.nvim_create_user_command('TestPrompt', function() vim.fn.confirm('Plugin question?', '&Yes\\n&No', 2) end, {})")
 command('TestPrompt')
 assert 'Plugin question?' in state['results'][-1]['body'],state['results'][-1]
 n.input('n<CR>'); wait()
 command('let g:secret = inputsecret("Password: ")')
 n.input('secret123'); wait()
 assert 'secret123' not in repr(state),state
 n.input('<CR>'); wait()
 assert n.vars['secret']=='secret123'
 command('echo "last_output"')
 command('IslandOutput')
 assert state['results'][-1]['body']=='last_output',state['results'][-1]
 n.command('IslandCommandLine off')
 previous=len(state['results'])
 command('echo "native_after_off"')
 assert len(state['results'])==previous
 assert any('native_after_off' in row for row in screen()),screen()
 print('PASS: full output, Unicode, Lua, plugins, async, shell, history, prompts, secret masking, reopening and fallback')
finally:
 try:n.command('qa!')
 except Exception:pass
 fixture.cleanup()
