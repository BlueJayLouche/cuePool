"""Standalone Nodel reference recipe. Configure a CuePool IPv4 address/port.

The UI action carries a panel ID and sequence to correlate local pending input
with the accepted volume event; these are separate from the OSC request ID.
"""
from java.lang import System
from java.security import SecureRandom
from java.util import UUID
from volume import VolumeController

param_ipAddress = Parameter({'title': 'CuePool IPv4 address',
    'schema': {'type': 'string', 'hint': '127.0.0.1'}})
param_port = Parameter({'title': 'CuePool OSC receive port',
    'schema': {'type': 'integer', 'hint': '9000'}})

local_event_ConfirmedVolume = LocalEvent({'schema': {'type': 'number'},
    'desc': 'Last correlated CuePool master-volume reply in dB; see VolumeAvailable for freshness.'})
local_event_VolumeAvailable = LocalEvent({'schema': {'type': 'boolean'}})
local_event_VolumeState = LocalEvent({'schema': {'type': 'object'}})

controller = None
udp = None
poller = None
destination = None


def publish(state):
    local_event_VolumeAvailable.emit(state['available'])
    if state['confirmedDb'] is not None:
        local_event_ConfirmedVolume.emit(state['confirmedDb'])
    local_event_VolumeState.emit(state)


@local_action({'schema': {'type': 'object', 'required': ['db', 'client', 'sequence', 'final', 'session'],
    'properties': {'db': {'type': 'number', 'minimum': -96, 'maximum': 12},
                   'client': {'type': 'string'}, 'sequence': {'type': 'integer'},
                   'final': {'type': 'boolean'}, 'session': {'type': 'string'}}}})
def Volume(arg):
    if controller is None:
        return
    if arg['session'] != controller.session:
        raise ValueError('Nodel restarted; refresh volume before adjusting it')
    client = arg['client']
    sequence = arg['sequence']
    final = arg['final']
    if not client or len(client) > 128 or sequence < 0 or int(sequence) != sequence:
        raise ValueError('Invalid panel ID or sequence')
    if final is not True and final is not False:
        raise ValueError('final must be boolean')
    controller.submit(arg['db'], client, sequence, final)


def received(source, data):
    if controller and str(source) == destination:
        controller.receive(data.encode('latin-1'))


def ready():
    if controller:
        controller.connected()


def main():
    global controller, udp, poller, destination
    host = (param_ipAddress or '127.0.0.1').strip()
    parts = host.split('.')
    if len(parts) != 4 or any(not p.isdigit() or int(p) > 255 for p in parts):
        raise ValueError('Use the CuePool IPv4 address, not a hostname')
    port = int(param_port or 9000)
    if not 1 <= port <= 65535:
        raise ValueError('OSC receive port must be between 1 and 65535')
    destination = '.'.join(str(int(p)) for p in parts) + ':' + str(port)
    controller = VolumeController(
        lambda packet: udp.send(packet.decode('latin-1')),
        publish, lambda: System.nanoTime() / 1000000000.0,
        SecureRandom().nextInt(), str(UUID.randomUUID()))
    # One bound socket sends requests AND receives replies on its ephemeral
    # source port. CuePool's configured OSC transmit destination is irrelevant.
    udp = UDP(source='0.0.0.0:0', dest=destination, ready=ready, received=received)
    poller = Timer(lambda: controller.tick(), 0.05)


@at_cleanup
def cleanup():
    if poller:
        poller.stop()
    if udp:
        udp.close()
