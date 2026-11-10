"""Run with OFFLINE_TEST_REPLAY pointing at fresh real-loop evidence."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import verify

ROOT = Path(__file__).resolve().parent

class GateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.path = Path(os.environ['OFFLINE_TEST_REPLAY'])
        cls.rows = json.loads(cls.path.read_text())
        cls.signals = [tuple(map(int, s.split(','))) for s in (ROOT/'fixture/signals.stdout.txt').read_text().splitlines()]
        cls.reference = json.loads((ROOT/'fixture/reconstruction.json').read_text())

    def test_positive_then_individual_mutations_actual_cli_both_modes(self):
        self.assertEqual(verify.compare(self.rows, self.signals, self.reference), 206)
        signal = next(i for i, (_, s) in enumerate(self.signals) if s)
        reject = next(i for i, r in enumerate(self.rows) if r['last_order'] and r['last_order']['outcome']=='rejected')
        hold = 64
        self.assertEqual(self.signals[hold][1], 0)
        mutations = []
        def add(name, index, path, value):
            bad = copy.deepcopy(self.rows)
            target = bad[index]
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = value
            mutations.append((name, bad))
        for key, value in [('accepted',9),('rejected',9),('accepted',True),('rejected',-1),('fills',1),('equity',float('nan')),('cash',float('inf')),('average',999),('fees',999),('position',99)]:
            add('hold_'+key+str(value),hold,[key],value)
        add('fills999',signal,['fills'],999)
        add('extra_filled_quantity',signal,['last_order','filled_quantity'],99)
        add('reject_price',reject,['last_order','price'],999)
        for key,value in [('price',999),('filled_quantity',99),('status','New'),('id',True),('seq',9),('order_type','Limit'),('fee',999),('symbol','HYPE-USD_UM_XPERP-310523')]:
            add('response_'+key,signal,['events',1,key],value)
        for key,value in [('quantity',99),('raw_price',999),('order_type','Limit'),('side','Buy')]:
            add('request_'+key,signal,['events',0,key],value)
        add('missing_events',signal,['events'],[])
        add('extra_events',signal,['events'],self.rows[signal]['events']*2)
        add('hold_events',hold,['events'],self.rows[signal]['events'])
        bad=copy.deepcopy(self.rows); del bad[signal]['cash']; mutations.append(('missing_cash',bad))
        bad=copy.deepcopy(self.rows); bad[0]['unknown']=0; mutations.append(('unknown_field',bad))
        mutations.extend([('empty',[]),('truncated',self.rows[:-1])])
        with tempfile.TemporaryDirectory(prefix='offline-mutations-') as directory:
            for flags in ([],['-O']):
                good=subprocess.run([sys.executable,*flags,str(ROOT/'verify.py'),str(self.path)],capture_output=True,text=True)
                self.assertEqual(good.returncode,0,good.stderr)
                for name,bad in mutations:
                    with self.subTest(mode=flags,name=name):
                        path=Path(directory)/(name+'.json')
                        path.write_text(json.dumps(bad))
                        run=subprocess.run([sys.executable,*flags,str(ROOT/'verify.py'),str(path)],capture_output=True,text=True)
                        self.assertNotEqual(run.returncode,0,name)
                        self.assertIn('ValueError',run.stderr)
        print(f'MUTATIONS_REJECTED={len(mutations)} per mode; normal and -O')

    def test_reference_unknown_missing_and_nonfinite_fail_closed(self):
        for kind in ('unknown', 'missing', 'nan', 'empty'):
            with self.subTest(kind=kind):
                reference=copy.deepcopy(self.reference)
                if kind == 'unknown':
                    reference['ledger'][0]['unknown']=1
                elif kind == 'missing':
                    del reference['ledger'][0]['fill']
                elif kind == 'nan':
                    reference['equity']=float('nan')
                else:
                    reference['ledger']=[]
                with self.assertRaises((ValueError, KeyError)):
                    verify.compare(self.rows,self.signals,reference)

    def test_export_no_clobber_and_default_isolation(self):
        with tempfile.TemporaryDirectory(prefix='offline-export-') as directory:
            path=Path(directory)/'comparison.json'
            cmd=[sys.executable,str(ROOT/'verify.py'),str(self.path),'--export',str(path)]
            first=subprocess.run(cmd,capture_output=True,text=True)
            self.assertEqual(first.returncode,0,first.stderr)
            original=path.read_bytes()
            second=subprocess.run(cmd,capture_output=True,text=True)
            self.assertNotEqual(second.returncode,0)
            self.assertEqual(path.read_bytes(),original)
            self.assertNotEqual(first.stdout.split('COMPARISON=')[-1],second.stdout.split('COMPARISON=')[-1])

if __name__ == '__main__':
    unittest.main()
