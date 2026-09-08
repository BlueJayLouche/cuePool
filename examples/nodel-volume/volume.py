"""CuePool volume polling, compatible with Nodel's Jython 2.5 and Python 3.

One request is outstanding at a time. Slider intent, accepted OSC state and
connection availability are distinct; request IDs are opaque, not timestamps.
"""
import struct


def osc_string(text):
    data = text.encode('ascii') + struct.pack('B', 0)
    return data + struct.pack('B', 0) * ((-len(data)) % 4)


STATE_PREFIX = osc_string('/qplayer/volume/state') + osc_string(',if')


def request_packet(request_id, db=None):
    if db is None:
        return (osc_string('/qplayer/volume/get') + osc_string(',i') +
                struct.pack('>i', request_id))
    return (osc_string('/qplayer/volume') + osc_string(',fi') +
            struct.pack('>fi', db, request_id))


def read_state(data):
    if len(data) != len(STATE_PREFIX) + 8 or not data.startswith(STATE_PREFIX):
        return None
    request_id, db = struct.unpack('>if', data[len(STATE_PREFIX):])
    if not (-96.0 <= db <= 12.0):
        return None
    return request_id, db


class VolumeController(object):
    POLL_SECONDS = 1.0
    TIMEOUT_SECONDS = 1.0
    DRAG_SECONDS = 0.1
    MAX_MISSES = 3

    def __init__(self, send, publish, now, initial_id, session):
        self.send = send
        self.publish = publish
        self.now = now
        self.next_id = initial_id
        self.session = session
        self.confirmed = None
        self.available = False
        self.desired = None
        self.intent = None
        self.flight = None
        self.misses = 0
        self.next_poll = 0.0
        self.last_send = -1.0
        self.revision = 0

    def snapshot(self):
        return {'session': self.session, 'revision': self.revision,
                'available': self.available, 'confirmedDb': self.confirmed,
                'pendingDb': self.desired['db'] if self.desired else None,
                'client': self.intent['client'] if self.intent else None,
                'sequence': self.intent['sequence'] if self.intent else None,
                'final': self.intent['final'] if self.intent else False}

    def emit(self):
        self.revision += 1
        self.publish(self.snapshot())

    def connected(self):
        # UDP ready means the local socket exists, not that CuePool is alive.
        # Never replay a remembered room level on startup/reconnect.
        self.flight = None
        self.desired = None
        self.available = False
        self.next_poll = self.now()
        self.emit()
        self.tick()

    def submit(self, db, client, sequence, final):
        db = float(db)
        if not (-96.0 <= db <= 12.0):
            raise ValueError('Volume must be finite and between -96 and +12 dB')
        if (self.intent and client == self.intent['client'] and
                sequence <= self.intent['sequence']):
            return
        self.intent = {'db': db, 'client': client, 'sequence': sequence, 'final': final}
        self.desired = self.intent
        self.emit()
        self.tick()

    def tick(self):
        now = self.now()
        if self.flight:
            if now - self.flight['sent'] < self.TIMEOUT_SECONDS:
                return
            self.flight = None
            self.misses += 1
            if self.misses >= self.MAX_MISSES:
                self.available = False
                self.desired = None
            self.emit()

        intent = self.desired
        if intent:
            # Coalesce previews while waiting for feedback. A final action is
            # always sent, even when it has the same dB as the last preview.
            if not intent['final'] and now - self.last_send < self.DRAG_SECONDS:
                return
        elif now < self.next_poll:
            return

        request_id = self.next_id
        self.next_id = -2147483648 if request_id == 2147483647 else request_id + 1
        self.flight = {'id': request_id, 'sent': now, 'intent': intent}
        self.next_poll = now + self.POLL_SECONDS
        self.last_send = now
        self.send(request_packet(request_id, intent['db'] if intent else None))

    def receive(self, data):
        state = read_state(data)
        if state is None or self.flight is None or state[0] != self.flight['id']:
            return False
        sent_intent = self.flight['intent']
        self.flight = None
        self.confirmed = state[1]
        self.available = True
        self.misses = 0
        if sent_intent is self.desired:
            self.desired = None
        self.emit()
        self.tick()
        return True
