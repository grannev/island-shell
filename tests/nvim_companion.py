import pynvim, socket, threading, json, time, os, tempfile
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
fixture = tempfile.TemporaryDirectory(prefix="island-input-")
path=str(Path(fixture.name) / 'island-shell-test.sock')
config=Path(fixture.name)/'island-shell'; config.mkdir(); (config/'nvim-token').write_text('test')
try: os.unlink(path)
except FileNotFoundError: pass
listener=socket.socket(socket.AF_UNIX); listener.bind(path); listener.listen()
state={'reply':b'ok: cmdline\n','last':None}
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
   if payload['event']=='cmdline': state['last']=payload
   elif payload['event']=='state': state['editor']=payload
   if state['reply']:
    try: conn.sendall(state['reply'])
    except BrokenPipeError: pass
   else: time.sleep(.3)
threading.Thread(target=serve,daemon=True).start()
n=pynvim.attach('child',argv=['nvim','--clean','--embed','-n'])
n.ui_attach(100,35,rgb=True,ext_linegrid=True)
n.exec_lua('''
local runtime, socket = ...
vim.opt.rtp:prepend(runtime)
vim.o.ruler = false
vim.o.showcmd = false
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
vim.env.XDG_CONFIG_HOME = vim.fn.fnamemodify(socket, ':h')
vim.env.XDG_RUNTIME_DIR = vim.env.XDG_CONFIG_HOME
vim.env.WAYLAND_DISPLAY = 'test'
require('island').setup({toasts={saved=false,progress=false,git=false}})
''', str(ROOT / 'nvim'), path)
def wait(ms=400): n.exec_lua('vim.wait(...)',ms)
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
   elif group[0]=='grid_clear': grid.clear()
 n._session._pending_messages.clear()
 return [''.join(grid.get((row,col),' ') for col in range(100)) for row in range(35)]
try:
 n.input(':let g:island_result = "готово"'); wait()
 assert n.eval('getcmdline()')=='let g:island_result = "готово"'
 assert state['last']['command']['text']=='let g:island_result = "готово"',state['last']
 assert event()['handled'], n.exec_lua('return _G.events')
 assert screen()[-1].strip() == '', screen()[-1]
 n.input('<Left>'); wait()
 assert state['last']['command']['pos']==len('let g:island_result = "готово'.encode()),state['last']
 n.input('<End><CR>'); wait()
 assert n.vars['island_result']=='готово'
 assert state['last']['command'] is None
 n.input('/abc'); wait()
 assert state['last']['command']['prefix']=='/'
 n.input('<Esc>'); wait()
 assert n.eval('mode()')=='n'
 n.input(':echo "fallback"'); wait()
 state['reply']=b'fallback\n'; wait(650)
 assert not n.exec_lua('return _G.attached')
 assert 'echo "fallback"' in screen()[-1], screen()[-1]
 assert n.eval('getcmdline()')=='echo "fallback"'
 state['reply']=b'ok: cmdline\n'; wait(650)
 assert event()['handled'],event()
 state['reply']=None; wait(700)
 assert not n.exec_lua('return _G.attached')
 assert 'echo "fallback"' in screen()[-1], screen()[-1]
 n.input('<Esc>'); wait()
 state['reply']=b'ok: cmdline\n'
 wait(1500)
 n.exec_lua('''vim.api.nvim_buf_set_lines(0,0,-1,false,{'a','b','c'})
 vim.diagnostic.set(vim.api.nvim_create_namespace('test'),0,{{lnum=2,col=0,severity=2,message='Warning'},{lnum=1,col=0,severity=1,message='Error'}})''')
 wait(800)
 assert [d['line'] for d in state['editor']['diagnostic_items']]==[2,3], state['editor']
 n.exec_lua('require("island").jump(vim.api.nvim_get_current_buf(), 2, 1)')
 assert list(n.current.window.cursor)==[2,0]
 print('PASS: Unicode, caret movement, execution, search, Escape, hidden island, recovery, timeout, diagnostic sorting and exact jump')
finally:
 try:n.command('qa!')
 except Exception:pass
 fixture.cleanup()
