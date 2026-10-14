#!/usr/bin/env python3
"""Local Chromium fixture. No extensions, real credentials or shared browser profile."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import base64
import hashlib
import json
import os
import ssl
import subprocess
import sys

mode = sys.argv[1] if len(sys.argv) > 1 else "browser"
protected = mode.startswith("browser-password")
secure = protected and mode != "browser-password-http"
cancel = mode.endswith("cancel")
initial = "" if protected else "при" if mode == "browser-unicode" else "43556" if mode == "browser-t9" else "hel"
html = ('''<!doctype html><title>Aqua browser test</title><style>
body {background:#edf2fa;color:#1d2534;font:18px sans-serif;padding:70px;}
input {font:24px sans-serif;padding:16px;border:1px solid #789;border-radius:14px;width:580px;}
</style><h1>Native suggestions, no extension</h1><p>Local browser fixture</p>
<input id="entry" type="TYPE" value="INITIAL" autofocus><p>Tab accepts the native suggestion.</p>
<input id="other" type="OTHER_KIND" placeholder="Cancellation target">
<script>const e=document.getElementById('entry'); let last=e.value;
e.focus();e.setSelectionRange(e.value.length,e.value.length);
fetch('/ready',{method:'POST'});
e.addEventListener('input',()=>{fetch('/edit',{method:'POST',body:JSON.stringify({
text:PROTECTED?undefined:e.value,length:[...e.value].length,chars:[...e.value].length-[...last].length,ns:performance.now()})});last=e.value;
if(CANCEL)document.getElementById('other').focus();});
</script>'''.replace("TYPE", "password" if protected else "text").replace("INITIAL", initial)
    .replace("OTHER_KIND", "password" if mode == "browser-password-protected-cancel" else "text")
    .replace("PROTECTED", str(protected).lower()).replace("CANCEL", str(cancel).lower())).encode()

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_GET(self):
        self.send_response(200); self.send_header("Content-Type", "text/html; charset=utf-8"); self.end_headers();self.wfile.write(html)
    def do_POST(self):
        data=self.rfile.read(int(self.headers.get('Content-Length',0)))
        if self.path == '/ready':
            print('INPUT_ENABLED',flush=True)
        elif self.path == '/edit':
            value=json.loads(data)
            if value['chars'] > 0:
                print(f"INSERT chars={value['chars']} ns={value['ns']}",flush=True)
                if cancel: print('INPUT_DISABLED_AFTER_FIRST',flush=True)
            if protected: print('PASSWORD_LENGTH='+str(value['length']),flush=True)
            else: print('TEXT='+value['text'],flush=True)
        self.send_response(204);self.end_headers()

server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
runtime=Path(os.environ['XDG_RUNTIME_DIR'])
options=[]
if secure:
    cert, key = runtime/'browser-cert.pem', runtime/'browser-key.pem'
    subprocess.run(['openssl','req','-x509','-nodes','-newkey','rsa:2048','-subj','/CN=127.0.0.1',
        '-addext','subjectAltName=IP:127.0.0.1','-days','1','-keyout',str(key),'-out',str(cert)],
        check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    key.chmod(0o600)
    pub=subprocess.check_output(['openssl','x509','-in',str(cert),'-pubkey','-noout'])
    der=subprocess.run(['openssl','pkey','-pubin','-outform','der'],input=pub,stdout=subprocess.PIPE,check=True).stdout
    options.append('--ignore-certificate-errors-spki-list='+base64.b64encode(hashlib.sha256(der).digest()).decode())
    context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(cert,key)
    server.socket=context.wrap_socket(server.socket,server_side=True)
    origin='https://127.0.0.1:'+str(server.server_port)
    subprocess.run(['gnome-keyring-daemon','--unlock','--components=secrets'],input=b'aqua-test-keyring',
        check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    fixture=Path(__file__).resolve().parents[1]/'target/release/examples/password_fixture'
    subprocess.run([str(fixture)],env={**os.environ,'AQUA_TEST_KEYRING':'1','AQUA_TEST_ORIGIN':origin},check=True)
else:
    origin='http://127.0.0.1:'+str(server.server_port)
profile=runtime/'browser-test'
environment={key:value for key,value in os.environ.items() if key not in ('GTK_IM_MODULE','WAYLAND_DEBUG')}
process=subprocess.Popen(['chromium','--ozone-platform=wayland','--force-renderer-accessibility',
    '--no-first-run','--no-default-browser-check','--disable-gpu','--disable-dev-shm-usage',
    '--disable-background-networking','--disable-component-update','--password-store=basic',
    '--user-data-dir='+str(profile),'--app='+origin,*options],env=environment,
    stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
try:
    server.serve_forever()
finally:
    process.terminate();server.server_close()
