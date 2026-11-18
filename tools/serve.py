"""Static file server for local checks: python3 tools/serve.py <port> [folder]
Like `python3 -m http.server --bind 127.0.0.1`, but with a deep listen queue. The page asks for
about twenty module files at once, and http.server's default queue of 5 makes macOS reset some
of those connections, which looks like a page that never finishes loading."""
import functools, http.server, os, sys
class Server(http.server.ThreadingHTTPServer):
    request_queue_size = 128
    daemon_threads = True
class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a): pass
port = int(sys.argv[1]); folder = os.path.abspath(sys.argv[2] if len(sys.argv) > 2 else '.')
Server(('127.0.0.1', port), functools.partial(Quiet, directory=folder)).serve_forever()
