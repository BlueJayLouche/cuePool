"""Run the example on a disposable Nodel host against a loopback OSC simulator.

python3 examples/nodel-volume/tests/smoke_nodel.py --jar /path/to/nodelhost.jar
Add --hold to keep the verified host open for browser inspection (Ctrl-C cleans up).
No venue hosts, audio devices or existing Nodel directories are used.
"""
import argparse
import json
import pathlib
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request

EXAMPLE = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(EXAMPLE))
from volume import STATE_PREFIX, osc_string


class Simulator:
    def __init__(self):
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.bind(('127.0.0.1', 0))
        self.socket.settimeout(0.1)
        self.volume = -8.0
        self.enabled = True
        self.delay = 0.0
        self.sets = []
        self.stop = False
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        get = osc_string('/qplayer/volume/get') + osc_string(',i')
        set_ = osc_string('/qplayer/volume') + osc_string(',fi')
        while not self.stop:
            try:
                data, source = self.socket.recvfrom(256)
            except socket.timeout:
                continue
            if not self.enabled:
                continue
            if data.startswith(set_) and len(data) == len(set_) + 8:
                self.volume, request_id = struct.unpack('>fi', data[len(set_):])
                self.sets.append(self.volume)
            elif data.startswith(get) and len(data) == len(get) + 4:
                request_id, = struct.unpack('>i', data[len(get):])
            else:
                raise AssertionError('Unexpected OSC request: %r' % data)
            response = STATE_PREFIX + struct.pack('>if', request_id, self.volume)
            if self.delay:
                time.sleep(self.delay)
            self.socket.sendto(response, source)

    def close(self):
        self.stop = True
        self.thread.join(2)
        self.socket.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jar', required=True, type=pathlib.Path)
    parser.add_argument('--hold', action='store_true')
    args = parser.parse_args()
    simulator = Simulator()
    try:
        with tempfile.TemporaryDirectory(prefix='cuepool-nodel-volume-') as directory:
            root = pathlib.Path(directory)
            node = root / 'nodes' / 'CuePool Volume Test'
            shutil.copytree(EXAMPLE, node, ignore=shutil.ignore_patterns('tests', '__pycache__', 'README.md'))
            (node / 'nodeConfig.json').write_text(json.dumps({'paramValues': {
                'ipAddress': '127.0.0.1', 'port': simulator.socket.getsockname()[1]}}))
            with socket.socket() as probe:
                probe.bind(('127.0.0.1', 0))
                port = probe.getsockname()[1]
            (root / 'bootstrap.json').write_text(json.dumps({
                'NodelHostPort': port, 'networkInterfaces': ['lo0'], 'disableAdvertisements': True}))
            log = open(root / 'host.log', 'w+')
            process = subprocess.Popen(['java', '-jar', str(args.jar.resolve())], cwd=root,
                                       stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT)
            base = 'http://127.0.0.1:%d/REST/nodes/CuePool%%20Volume%%20Test/' % port

            def request(path, body=None):
                data = None if body is None else json.dumps(body).encode()
                req = urllib.request.Request(base + path, data=data,
                                             headers={'Content-Type': 'application/json'})
                with urllib.request.urlopen(req, timeout=3) as response:
                    return json.load(response)

            def wait_state(predicate, timeout=10):
                deadline = time.monotonic() + timeout
                last = None
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise AssertionError('Nodel exited')
                    try:
                        last = request('events')['VolumeState']['arg']
                        if last and predicate(last):
                            return last
                    except (OSError, KeyError, TypeError):
                        pass
                    time.sleep(0.1)
                raise AssertionError('Timed out waiting for volume state: %r' % last)

            try:
                state = wait_state(lambda s: s['available'] and s['confirmedDb'] == -8, 35)
                assert 'Volume' in request('actions')
                session = state['session']

                def set_volume(db, sequence, final):
                    request('actions/Volume/call', {'arg': {'db': db, 'client': 'smoke',
                        'sequence': sequence, 'final': final, 'session': session}})

                simulator.delay = 0.25
                for sequence in range(1, 21):
                    set_volume(-sequence, sequence, False)
                set_volume(-24, 21, True)
                wait_state(lambda s: s['confirmedDb'] == -24 and s.get('pendingDb') is None)
                assert len(simulator.sets) < 21, simulator.sets
                simulator.delay = 0

                before = len(simulator.sets)
                simulator.volume = -16  # External GUI edit / show load.
                wait_state(lambda s: s['confirmedDb'] == -16)
                assert len(simulator.sets) == before, 'Feedback caused another setter'

                simulator.enabled = False
                set_volume(-30, 22, True)
                wait_state(lambda s: not s['available'] and s.get('pendingDb') is None)
                simulator.volume = -4  # CuePool restarts at the saved show level.
                simulator.enabled = True
                wait_state(lambda s: s['available'] and s['confirmedDb'] == -4)
                assert len(simulator.sets) == before, 'Reconnection replayed an old drag'

                request('restart', {})
                state = wait_state(lambda s: s['session'] != session and s['available'])
                assert state['confirmedDb'] == -4
                assert state.get('pendingDb') is None
                console = request('console?from=0&max=100')
                errors = [entry for entry in console if entry.get('console') == 'err']
                assert not errors, errors
                print('PASS: real Nodel startup, correlated drag, external edits, timeouts, reconnect and node restart.', flush=True)
                print('Browser URL: http://127.0.0.1:%d/nodes/CuePool%%20Volume%%20Test/' % port, flush=True)
                if args.hold:
                    # Some automation browsers block XML navigation. Render the
                    # actual served XSL into HTML for the same DOM/JS UI checks.
                    if shutil.which('xsltproc'):
                        xsl = root / 'xslt'
                        xsl.mkdir()
                        for name in ['index.xsl', 'templates.xsl']:
                            url = 'http://127.0.0.1:%d/nodes/CuePool%%20Volume%%20Test/v1/%s' % (port, name)
                            (xsl / name).write_bytes(urllib.request.urlopen(url).read())
                        subprocess.run(['xsltproc', '-o', str(node / 'content' / 'preview.html'),
                                        str(xsl / 'index.xsl'), str(node / 'content' / 'index.xml')], check=True)
                        print('Transformed preview: http://127.0.0.1:%d/nodes/CuePool%%20Volume%%20Test/preview.html' % port, flush=True)
                    print('Holding disposable host for browser checks; Ctrl-C cleans up.', flush=True)
                    while True:
                        time.sleep(1)
            except Exception:
                try:
                    print('Node console:', request('console?from=0&max=100'), flush=True)
                except Exception:
                    pass
                log.flush()
                log.seek(0)
                print(log.read()[-6000:], flush=True)
                raise
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                log.close()
                process.stdin.close()
    finally:
        simulator.close()


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        pass
