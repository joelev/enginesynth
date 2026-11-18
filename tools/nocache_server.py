"""Static file server that tells the browser never to cache (for local previews).
usage: nocache_server.py <port> <directory>"""
import functools, http.server, sys
class H(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cache-Control', 'no-store, max-age=0')
        super().end_headers()
    def log_message(self, *a): pass
class S(http.server.ThreadingHTTPServer):
    request_queue_size = 128  # the page asks for about twenty module files at once
    daemon_threads = True
S(('127.0.0.1', int(sys.argv[1])), functools.partial(H, directory=sys.argv[2])).serve_forever()
