import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.compare_world_digests import FORMAT, REQUIRED, compare


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


class WorldDigestsTest(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.header = dict(format=FORMAT, mode='serial', seed=12345,
                           inputs_sha256=digest(['move east', 'save']),
                           initial_state_sha256=digest('initial world'),
                           dimensions=['overworld', 'nether'], ticks=2)
        self.records = [dict(tick=tick, dimension=dimension,
                             components={key: digest([tick, dimension, key]) for key in REQUIRED})
                        for tick in (1, 2) for dimension in ('overworld', 'nether')]

    def write(self, name, mode, records=None, **header):
        path = self.root / name
        path.write_text('\n'.join(json.dumps(row) for row in [
            dict(self.header, mode=mode, **header),
            *(self.records if records is None else records),
        ]) + '\n')
        return path

    def check(self, records=None, **header):
        return compare(self.write('serial', 'serial'),
                       self.write('parallel', 'regions', records, **header))

    def test_matches_all_ticks_and_dimensions(self):
        self.assertEqual(self.check(), 4)

    def test_reports_first_rng_divergence(self):
        self.records[2]['components']['random'] = digest(['a different RNG state'])
        serial = self.write('serial', 'serial')
        self.records[2]['components']['random'] = digest(['another RNG state'])
        with self.assertRaisesRegex(ValueError, 'tick 2, overworld, random'):
            compare(serial, self.write('parallel', 'regions'))

    def test_rejects_workload_mismatch(self):
        with self.assertRaisesRegex(ValueError, 'workload mismatch: seed'):
            self.check(seed=999)

    def test_rejects_different_initial_world(self):
        with self.assertRaisesRegex(ValueError, 'workload mismatch: initial_state_sha256'):
            self.check(initial_state_sha256=digest('different world'))

    def test_rejects_partial_or_reordered_transcripts(self):
        for records in ([], self.records[2:], self.records[:-1],
                        [self.records[1], self.records[0], *self.records[2:]],
                        [self.records[0], self.records[0], *self.records[2:]]):
            with self.subTest(records=records), self.assertRaises(ValueError):
                self.check(records)

    def test_rejects_incomplete_components(self):
        del self.records[0]['components']['entities']
        with self.assertRaisesRegex(ValueError, 'incomplete simulation components'):
            self.check()

    def test_rejects_serial_as_parallel(self):
        with self.assertRaisesRegex(ValueError, 'expected .* / regions'):
            compare(self.write('one', 'serial'), self.write('two', 'serial'))

    def test_queue_order_changes_its_digest(self):
        self.records[0]['components']['scheduled_block_ticks'] = digest(['a', 'b'])
        serial = self.write('serial', 'serial')
        self.records[0]['components']['scheduled_block_ticks'] = digest(['b', 'a'])
        with self.assertRaisesRegex(ValueError, 'scheduled_block_ticks'):
            compare(serial, self.write('parallel', 'regions'))


if __name__ == '__main__':
    unittest.main()
