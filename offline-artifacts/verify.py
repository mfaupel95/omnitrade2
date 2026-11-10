"""Fail-closed primary-event accounting gate; fixtures are read-only."""
import argparse
import csv
import hashlib
import json
import math
import tempfile
from pathlib import Path
ROOT = Path(__file__).resolve().parent
SYMBOL = 'HYPE-USD_UM_XPERP-310801'  # synthetic demo, not mainnet fixture 310523

def require(ok, message):
    if not ok:
        raise ValueError(message)

def schema(value, keys):
    require(type(value) is dict and set(value) == set(keys.split()), 'unknown/missing schema fields')

def number(value):
    require(type(value) in (int, float) and math.isfinite(value), 'nonfinite/non-numeric evidence')
    return value

def integer(value):
    require(type(value) is int and value >= 0, 'counter/identity must be nonnegative int, not bool')
    return value

def close(a, b):
    require(abs(number(a) - number(b)) <= 1e-9, f'numeric mismatch: {a!r} != {b!r}')

def compare(rows, signals, reference):
    require(type(rows) is list and len(rows) == len(signals) == 1440, 'incomplete candles')
    with (ROOT/'fixture/candles.csv').open() as source:
        marks = [float(c[4]) for c in list(csv.reader(source))[1:]]
    schema(reference, 'accepted rejected position closed_lots gross_realized fees net_realized unrealized equity ledger status candles first_ms last_ms input_sha256 fee_assumption slippage_assumption stop_applied forced_end_close')
    require(reference['status'] == 'partial_reconstruction_NOT_execution_parity', 'reference scope changed')
    require(integer(reference['candles']) == 1440 and integer(reference['first_ms']) == signals[0][0] and integer(reference['last_ms']) == signals[-1][0], 'reference time window')
    # Source-export digest metadata differs from the copied fixture digest.
    # The five-file hashes.json verifies actual fixture bytes in the CLI gate.
    manifest = json.loads((ROOT/'fixture/manifest.json').read_text())
    require(reference['input_sha256'] == manifest['sha256'], 'reference source digest metadata')
    close(reference['fee_assumption'], 0.0005)
    close(reference['slippage_assumption'], 0.0)
    require(reference['stop_applied'] is False and reference['forced_end_close'] is False, 'reference execution assumptions changed')
    for key in ('accepted', 'rejected', 'closed_lots'):
        integer(reference[key])
    for key in ('position', 'gross_realized', 'fees', 'net_realized', 'unrealized', 'equity'):
        number(reference[key])
    ledger = reference['ledger']
    require(type(ledger) is list and len(ledger) == sum(bool(s) for _, s in signals), 'incomplete reference')
    for event in ledger:
        require(type(event) is dict and type(event.get('accepted')) is bool, 'invalid reference acceptance')
        schema(event, 'timestamp_ms signal accepted position' + (' fill' if event['accepted'] else ''))
        integer(event['timestamp_ms'])
        require(type(event['signal']) is int and event['signal'] in (-1, 1), 'invalid reference signal')
        number(event['position'])
        if event['accepted']:
            number(event['fill'])
    position = cost = fees = realized = 0.0
    accepted = rejected = fills = orders = seq = 0
    last = None
    for i, (row, (timestamp, signal)) in enumerate(zip(rows, signals)):
        schema(row, 'timestamp_ms signal position fills accepted rejected price quantity cash average fees equity events last_order')
        require(integer(row['timestamp_ms']) == timestamp, 'timestamp mismatch')
        require(row['signal'] == {-1:'sell', 0:'hold', 1:'buy'}[signal], 'signal mismatch')
        for key in ('position','price','quantity','cash','average','fees','equity'):
            number(row[key])
        close(row['price'], marks[i])
        close(row['quantity'], 0.1)
        for key in ('accepted','rejected','fills'):
            integer(row[key])
        expected = ledger[orders] if signal else None
        ok = bool(expected and expected['accepted'])
        require(row['accepted'] == accepted + int(ok), 'unexpected accepted delta')
        require(row['rejected'] == rejected + int(bool(signal) and not ok), 'unexpected rejected delta')
        events = row['events']
        require(type(events) is list and len(events) == (2 if ok else 0), 'unexpected/missing primary events')
        if signal:
            require(expected['timestamp_ms'] == timestamp and expected['signal'] == signal and type(expected['accepted']) is bool, 'reference identity mismatch')
            order = row['last_order']
            schema(order, 'detail outcome price quantity side time_ms')
            integer(order['time_ms'])
            require(type(order['detail']) is str and order['side'] == row['signal'], 'status identity mismatch')
            require(order['outcome'] == ('placed' if ok else 'rejected'), 'status outcome mismatch')
            close(order['quantity'], 0.1)
            close(order['price'], marks[i])  # request/reference price, never a fill
            if not ok:
                require(order['detail'].startswith('Risk guard rejection: Positionsdeckel'), 'unknown rejection category')
            last = order
            orders += 1
        else:
            require(row['last_order'] == last, 'hold changed order evidence')
        if ok:
            request, response = events
            schema(request, 'seq kind id symbol side order_type quantity raw_price')
            schema(response, 'seq kind id symbol side order_type quantity price filled_quantity status fee')
            for event, kind in ((request, 'request'), (response, 'response')):
                seq += 1
                require(integer(event['seq']) == seq and event['kind'] == kind, 'event sequence/type mismatch')
                require(integer(event['id']) == accepted + 1, 'duplicate/wrong identity')
                require(event['symbol'] == SYMBOL and event['side'] == ('Buy' if signal == 1 else 'Sell') and event['order_type'] == 'Market', 'unknown event contract')
                close(event['quantity'], 0.1)
            require(request['raw_price'] is None, 'unexpected raw market price')
            require(response['status'] == 'Filled', 'fixture requires full fill')
            close(response['price'], expected['fill'])
            close(response['price'], marks[i])
            close(response['filled_quantity'], request['quantity'])
            q = number(response['filled_quantity']) * signal
            p = number(response['price'])
            fee = abs(q) * p * 0.0005
            close(response['fee'], fee)
            fees += fee
            # Signed inventory cost independent of stored fake average/cash.
            if position * q < 0:
                closed = min(abs(position), abs(q))
                realized += closed * (p - cost / position) * (1 if position > 0 else -1)
                if abs(q) <= abs(position):
                    cost *= (abs(position) - closed) / abs(position)
                else:
                    cost = (position + q) * p
            else:
                cost += q * p
            position += q
            if abs(position) < 1e-12:
                position = cost = 0.0
            fills += 1
        accepted += int(ok)
        rejected += int(bool(signal) and not ok)
        require(row['fills'] == fills, 'unexpected fills delta')
        close(row['position'], position)
        if signal:
            close(position, expected['position'])
        close(row['average'], cost / position if position else 0.0)
        close(row['fees'], fees)
        close(row['cash'], 50.0 + realized - fees)
        close(row['equity'], 50.0 + realized - fees + position * marks[i] - cost)
    require(orders == len(ledger), 'extra reference events')
    require(accepted == reference['accepted'] and rejected == reference['rejected'], 'final counters')
    close(position, reference['position'])
    close(fees, reference['fees'])
    close(rows[-1]['equity'], reference['equity'])
    return orders

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('replay', type=Path)
    parser.add_argument('--export', type=Path)
    args = parser.parse_args()
    fixture = ROOT/'fixture'
    hashes = json.loads((fixture/'hashes.json').read_text())
    require(len(hashes) == 5, 'missing fixture hashes')
    for name, expected in hashes.items():
        require(hashlib.sha256((fixture/name).read_bytes()).hexdigest() == expected, 'fixture hash: '+name)
    rows = json.loads(args.replay.read_text())
    signals = [tuple(map(int,line.split(','))) for line in (fixture/'signals.stdout.txt').read_text().splitlines()]
    reference = json.loads((fixture/'reconstruction.json').read_text())
    orders = compare(rows, signals, reference)
    report = dict(candles=len(rows), orders=orders, accepted=rows[-1]['accepted'], rejected=rows[-1]['rejected'], position=rows[-1]['position'], equity=rows[-1]['equity'], fixture_hashes_verified=len(hashes), replay_sha256=hashlib.sha256(args.replay.read_bytes()).hexdigest())
    scratch = Path(tempfile.mkdtemp(prefix='offline-verify-'))
    text = json.dumps(report, indent=2)+'\n'
    (scratch/'comparison.json').write_text(text)
    if args.export:
        # Atomic no-clobber publication; cross-volume export fails explicitly.
        import os
        os.link(scratch/'comparison.json', args.export)
    print(text + 'COMPARISON='+str(scratch/'comparison.json'))

if __name__ == '__main__':
    main()
