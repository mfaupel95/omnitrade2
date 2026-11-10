"""Verify every real-loop candle/order against frozen pre-recovery references."""
import copy
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent

def compare(rows, signals, reference):
    assert len(rows) == len(signals) == 1440
    ledger = iter(reference['ledger'])
    position = 0.0
    previous_accepted = previous_rejected = 0
    orders = 0
    for row, (timestamp, signal) in zip(rows, signals):
        assert row['timestamp_ms'] == timestamp
        assert row['signal'] == {-1:'sell', 0:'hold', 1:'buy'}[signal]
        if signal:
            expected = next(ledger)
            assert expected['timestamp_ms'] == timestamp
            assert expected['signal'] == signal
            accepted = row['accepted'] > previous_accepted
            assert accepted == expected['accepted']
            assert row['accepted'] - previous_accepted == int(accepted)
            assert row['rejected'] - previous_rejected == int(not accepted)
            order = row['last_order']
            assert order['side'] == ('buy' if signal == 1 else 'sell')
            assert abs(order['quantity'] - 0.1) < 1e-12
            assert order['outcome'] == ('placed' if accepted else 'rejected')
            if accepted:
                assert abs(order['price'] - expected['fill']) < 1e-10
            position = expected['position']
            orders += 1
        assert abs(row['position'] - position) < 1e-10
        previous_accepted, previous_rejected = row['accepted'], row['rejected']
    assert next(ledger, None) is None
    assert rows[-1]['accepted'] == reference['accepted']
    assert rows[-1]['rejected'] == reference['rejected']
    assert abs(rows[-1]['equity'] - reference['equity']) < 1e-9
    return orders

if __name__ == '__main__':
    fixture = ROOT/'fixture'
    hashes = json.loads((fixture/'hashes.json').read_text())
    for name, expected in hashes.items():
        assert hashlib.sha256((fixture/name).read_bytes()).hexdigest() == expected, name
    rows = json.loads((ROOT/'hype-replay.json').read_text())
    signals = [tuple(map(int,line.split(','))) for line in (fixture/'signals.stdout.txt').read_text().splitlines()]
    reference = json.loads((fixture/'reconstruction.json').read_text())
    orders = compare(rows, signals, reference)
    bad = copy.deepcopy(rows)
    bad[60]['position'] += 0.1
    try:
        compare(bad, signals, reference)
    except AssertionError:
        negative_detected = True
    else:
        raise AssertionError('divergence not detected')
    report = dict(candles=len(rows), orders=orders, accepted=rows[-1]['accepted'],rejected=rows[-1]['rejected'],position=rows[-1]['position'],equity=rows[-1]['equity'],negative_control_detected=negative_detected,fixture_hashes_verified=len(hashes),replay_sha256=hashlib.sha256((ROOT/'hype-replay.json').read_bytes()).hexdigest())
    (ROOT/'comparison.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))
