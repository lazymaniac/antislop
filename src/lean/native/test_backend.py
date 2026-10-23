#!/usr/bin/env python3
"""Offline native-provider protocol fixture. Only Python's standard library."""
import base64
import hashlib
import http.server
import json
import os
import socket
import struct
import sys
import threading

IDENTITY = 'session_' + str(os.getpid())


def record(method, params=None):
    with open('native-calls.jsonl', 'a', encoding='utf8') as output:
        output.write(json.dumps({'pid': os.getpid(), 'method': method, 'params': params}) + '\n')


def exact(stream, size):
    data = b''
    while len(data) < size:
        chunk = stream.recv(size - len(data))
        if not chunk:
            raise EOFError()
        data += chunk
    return data


def websocket(stream):
    try:
        header = b''
        while not header.endswith(b'\r\n\r\n'):
            header += exact(stream, 1)
        key = next(line.split(b':', 1)[1].strip() for line in header.split(b'\r\n')
                   if line.lower().startswith(b'sec-websocket-key:'))
        accept = base64.b64encode(hashlib.sha1(key + b'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest())
        stream.sendall(b'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + b'\r\n\r\n')
        while True:
            first, second = exact(stream, 2)
            if first & 15 == 8:
                return
            size = second & 127
            if size == 126:
                size = struct.unpack('!H', exact(stream, 2))[0]
            elif size == 127:
                size = struct.unpack('!Q', exact(stream, 8))[0]
            mask = exact(stream, 4) if second & 128 else None
            data = exact(stream, size)
            if mask:
                data = bytes(byte ^ mask[i % 4] for i, byte in enumerate(data))
            request = json.loads(data)
            if 'id' not in request:
                continue
            method, params = request['method'], request.get('params', {})
            record(method, params)
            if method == 'thread/start' and os.path.exists('.antislop/unsupported-native-protocol'):
                data = json.dumps({'id': request['id'], 'error': {'code': -32601, 'message': 'method not found'}}).encode()
                stream.sendall(bytes([129, len(data)]) + data)
                continue
            result = {'thread': {'id': params.get('threadId', IDENTITY)}} if method.startswith('thread/') else {}
            data = json.dumps({'id': request['id'], 'result': result}).encode()
            frame = bytes([129, len(data)]) if len(data) < 126 else bytes([129, 126]) + struct.pack('!H', len(data))
            stream.sendall(frame + data)
    except (EOFError, OSError):
        pass
    finally:
        stream.close()


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        data = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'{}')
        record(self.path, data)
        if self.path == '/session' and os.path.exists('.antislop/unsupported-native-protocol'):
            self.send_error(404)
            return
        self.respond({'id': IDENTITY})

    def do_GET(self):
        record(self.path)
        self.respond({'id': self.path.rsplit('/', 1)[-1]})

    def respond(self, data):
        body = json.dumps(data).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if sys.argv[1] in ('resume', 'attach'):
    record('terminal_attached', {'identity': sys.argv[2:]})
    print('offline terminal ready', flush=True)
    for line in sys.stdin:
        print(line, end='', flush=True)
elif sys.argv[1] == 'app-server':
    path = sys.argv[sys.argv.index('--listen') + 1].removeprefix('unix://')
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(path)
    listener.listen()
    while True:
        stream, _ = listener.accept()
        threading.Thread(target=websocket, args=(stream,), daemon=True).start()
elif sys.argv[1] == 'serve':
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    print('http://127.0.0.1:' + str(server.server_address[1]), flush=True)
    server.serve_forever()
else:
    raise RuntimeError('unexpected native fixture invocation: ' + repr(sys.argv))
