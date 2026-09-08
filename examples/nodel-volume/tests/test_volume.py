import math
import pathlib
import struct
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from volume import STATE_PREFIX, VolumeController, osc_string, read_state


def reply(request_id, db):
    return STATE_PREFIX + struct.pack('>if', request_id, db)


class VolumeTests(unittest.TestCase):
    def setUp(self):
        self.now = 10.0
        self.sent = []
        self.states = []
        self.client = VolumeController(self.sent.append, self.states.append,
                                       lambda: self.now, 40, 'session-a')
        self.client.connected()

    def acknowledge(self, db=0.0):
        return self.client.receive(reply(self.client.flight['id'], db))

    def advance(self, seconds):
        self.now += seconds
        self.client.tick()

    def test_startup_and_one_second_polling_observe_external_changes(self):
        self.assertFalse(self.client.available)
        self.assertTrue(self.sent[-1].startswith(osc_string('/qplayer/volume/get')))
        self.acknowledge(-6.0)
        count = len(self.sent)
        self.advance(0.9)
        self.assertEqual(len(self.sent), count)
        self.advance(0.1)
        self.acknowledge(-20.0)  # GUI edit / newly loaded show.
        self.assertEqual(self.client.confirmed, -20.0)
        self.assertTrue(all(packet.startswith(osc_string('/qplayer/volume/get'))
                            for packet in self.sent))

    def test_rapid_drag_coalesces_previews_and_always_sends_final(self):
        self.acknowledge()
        for sequence in range(1, 101):
            self.client.submit(-sequence / 2.0, 'panel', sequence, False)
            self.advance(0.001)
        # One preview at the throttle boundary, not 100 packets.
        self.assertLessEqual(len(self.sent), 2)
        self.advance(0.01)
        first = self.client.flight['id']
        first_value = self.client.flight['intent']['db']
        self.client.submit(-50.0, 'panel', 101, True)
        self.client.receive(reply(first, first_value))
        self.assertEqual(self.client.desired['sequence'], 101)
        self.assertEqual(self.client.flight['intent']['db'], -50.0)
        self.acknowledge(-50.0)
        self.assertIsNone(self.client.desired)
        self.assertEqual(self.client.snapshot()['sequence'], 101)
        self.assertTrue(self.client.snapshot()['final'])

    def test_late_poll_and_old_set_confirmations_do_not_clear_newer_input(self):
        poll_id = self.client.flight['id']
        self.client.submit(-12.0, 'panel', 1, True)
        self.client.receive(reply(poll_id, 0.0))
        old_set_id = self.client.flight['id']
        self.client.submit(-24.0, 'panel', 2, True)
        self.client.receive(reply(old_set_id, -12.0))
        self.assertEqual(self.client.snapshot()['pendingDb'], -24.0)
        self.acknowledge(-24.0)
        self.assertFalse(self.client.receive(reply(old_set_id, -12.0)))
        self.assertEqual(self.client.confirmed, -24.0)
        self.assertIsNone(self.client.desired)

    def test_final_is_sent_even_if_it_matches_an_acknowledged_preview(self):
        self.acknowledge()
        self.advance(0.11)
        self.client.submit(-8.0, 'panel', 1, False)
        self.acknowledge(-8.0)
        count = len(self.sent)
        self.client.submit(-8.0, 'panel', 2, True)
        self.assertEqual(len(self.sent), count + 1)
        self.assertTrue(self.client.flight['intent']['final'])

    def test_three_misses_abandon_pending_then_reconnect_by_reading(self):
        self.acknowledge()
        self.client.submit(-30.0, 'panel', 1, True)
        old_id = self.client.flight['id']
        self.advance(1)
        self.advance(1)
        self.assertTrue(self.client.available)
        self.advance(1)
        self.assertFalse(self.client.available)
        self.assertIsNone(self.client.desired)
        self.assertIsNone(self.client.flight['intent'])
        self.assertFalse(self.client.receive(reply(old_id, -30.0)))
        self.acknowledge(-3.0)  # CuePool restarted with a saved show level.
        self.assertTrue(self.client.available)
        self.assertEqual(self.client.confirmed, -3.0)
        count = len(self.sent)
        self.client.connected()  # Nodel socket reconnected too.
        self.assertEqual(len(self.sent), count + 1)
        self.assertIsNone(self.client.flight['intent'])

    def test_request_ids_wrap_and_only_match_the_outstanding_request(self):
        self.acknowledge()
        self.client.next_id = 2147483647
        self.advance(1)
        self.assertEqual(self.client.flight['id'], 2147483647)
        self.acknowledge()
        self.advance(1)
        self.assertEqual(self.client.flight['id'], -2147483648)
        self.assertFalse(self.client.receive(reply(2147483647, 12)))
        self.acknowledge(-96)
        self.assertEqual(self.client.confirmed, -96)

    def test_malformed_and_nonfinite_feedback_is_ignored(self):
        for packet in [b'', reply(40, 2)[:-1], reply(40, 2) + b'\0',
                       reply(40, 13), reply(40, -97), reply(40, math.nan),
                       reply(40, math.inf), STATE_PREFIX.replace(b',if', b',ff') + struct.pack('>ff', 40, 1)]:
            self.assertIsNone(read_state(packet))
            self.assertFalse(self.client.receive(packet))
        self.assertIsNone(self.client.confirmed)

    def test_old_panel_actions_are_ignored_and_invalid_levels_rejected(self):
        self.acknowledge()
        self.client.submit(-24, 'panel', 2, True)
        self.client.submit(-12, 'panel', 1, False)
        self.assertEqual(self.client.desired['db'], -24)
        for db in [math.nan, math.inf, -97, 13]:
            with self.assertRaises(ValueError):
                self.client.submit(db, 'panel', 3, True)

    def test_nodel_restart_begins_with_fresh_correlation_and_no_saved_intent(self):
        old_id = self.client.flight['id']
        self.client = VolumeController(self.sent.append, self.states.append,
                                       lambda: self.now, 500, 'session-b')
        self.client.connected()
        self.assertFalse(self.client.receive(reply(old_id, -50)))
        self.acknowledge(-4)
        self.assertEqual(self.client.snapshot()['session'], 'session-b')
        self.assertIsNone(self.client.intent)


if __name__ == '__main__':
    unittest.main()
