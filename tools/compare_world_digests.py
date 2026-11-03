#!/usr/bin/env python3
"""Compare real serial/region post-tick digest transcripts (no scheduler is run).

Each JSONL transcript starts with a header containing format
`horizon-world-digests-v1`, mode (`serial` or `regions`), seed, inputs_sha256,
initial_state_sha256, dimensions (ordered), and ticks. Each following record
contains tick (1..ticks), dimension and a components object with SHA-256 digests of every REQUIRED component. Records
must include every dimension at every tick, in serial commit order.

Exporters must hash canonical simulation state at the same post-tick barrier.
Object keys may be canonicalized; order-sensitive queues, candidates, entities
and RNG states must retain simulation order. Include unloaded/pending state,
not just visible chunks. See issue #8018 for the scheduler/digest contract.
A region transcript must come from a real region runtime: this tool provides
no serial-as-parallel fallback. Missing inputs and malformed/incomplete traces
and duplicate JSON fields fail closed. Matching traces prove only the supplied workload, not all parity.
"""
import argparse
import json
from pathlib import Path
import re

FORMAT = 'horizon-world-digests-v1'
REQUIRED = frozenset({
    'blocks', 'block_entities', 'entities', 'scheduled_block_ticks',
    'scheduled_fluid_ticks', 'block_events', 'random', 'tickets', 'poi',
    'transfers',
})
SHA256 = re.compile(r'[0-9a-f]{64}')


def unique_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'duplicate JSON field: {key}')
        result[key] = value
    return result


def read_trace(path, mode):
    with Path(path).open() as stream:
        header = json.loads(next(stream), object_pairs_hook=unique_fields)
        if header.get('format') != FORMAT or header.get('mode') != mode:
            raise ValueError(f'{path}: expected {FORMAT} / {mode}')
        if type(header.get('seed')) is not int:
            raise ValueError(f'{path}: missing integer seed')
        ticks = header.get('ticks')
        if type(ticks) is not int or ticks < 1:
            raise ValueError(f'{path}: ticks must be positive')
        for key in ('inputs_sha256', 'initial_state_sha256'):
            if not isinstance(header.get(key), str) or not SHA256.fullmatch(header[key]):
                raise ValueError(f'{path}: missing {key}')
        dimensions = header.get('dimensions')
        if (not isinstance(dimensions, list) or not dimensions
                or not all(isinstance(d, str) and d for d in dimensions)
                or len(set(dimensions)) != len(dimensions)):
            raise ValueError(f'{path}: invalid ordered dimensions')
        records = []
        current = []
        previous_tick = 1
        for line in stream:
            record = json.loads(line, object_pairs_hook=unique_fields)
            tick, dimension = record.get('tick'), record.get('dimension')
            if (type(tick) is not int or tick not in (previous_tick, previous_tick + 1)
                    or tick > ticks or (not records and tick != 1)):
                raise ValueError(f'{path}: missing/out-of-order tick')
            if tick != previous_tick:
                if current != dimensions:
                    raise ValueError(f'{path}: dimension order/membership changed')
                current = []
                previous_tick = tick
            if not isinstance(dimension, str) or not dimension or dimension in current:
                raise ValueError(f'{path}: missing/duplicate dimension')
            current.append(dimension)
            components = record.get('components')
            if not isinstance(components, dict) or not REQUIRED <= components.keys():
                raise ValueError(f'{path}: incomplete simulation components')
            if not all(isinstance(v, str) and SHA256.fullmatch(v) for v in components.values()):
                raise ValueError(f'{path}: invalid component digest')
            records.append((tick, dimension, components))
        if not records or previous_tick != ticks or (dimensions and current != dimensions):
            raise ValueError(f'{path}: incomplete transcript')
        return header, records


def compare(serial, parallel):
    left_header, left = read_trace(serial, 'serial')
    right_header, right = read_trace(parallel, 'regions')
    for key in ('seed', 'inputs_sha256', 'initial_state_sha256', 'dimensions', 'ticks'):
        if left_header[key] != right_header[key]:
            raise ValueError(f'workload mismatch: {key}')
    if len(left) != len(right):
        raise ValueError('different record counts')
    for (tick, dimension, a), (other_tick, other_dimension, b) in zip(left, right):
        if (tick, dimension) != (other_tick, other_dimension):
            raise ValueError(f'tick {tick}: dimension/commit order differs')
        for component in sorted(a.keys() | b.keys()):
            if a.get(component) != b.get(component):
                raise ValueError(f'tick {tick}, {dimension}, {component}: '
                                 f'serial={a.get(component)} regions={b.get(component)}')
    return len(left)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True, type=Path)
    parser.add_argument('--parallel', required=True, type=Path)
    args = parser.parse_args()
    try:
        count = compare(args.serial, args.parallel)
    except (OSError, ValueError, StopIteration, TypeError, AttributeError) as error:
        parser.exit(1, f'digest comparison failed: {error}\n')
    print(f'Matched {count} post-tick dimension records.')


if __name__ == '__main__':
    main()
