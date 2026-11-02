#!/usr/bin/env python3
"""Alternate equal-work native runs; callers supply both binaries and public asset directory."""
import argparse, hashlib, json, os, platform, re, statistics, subprocess, resource
from pathlib import Path
p = argparse.ArgumentParser()
p.add_argument('--baseline', type=Path, required=True)
p.add_argument('--candidate', type=Path, required=True)
p.add_argument('--assets', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--pairs', type=int, default=3)
a = p.parse_args()
if a.pairs < 1: p.error('--pairs must be positive')
def sha(data): return hashlib.sha256(data).hexdigest()
workloads = [('hello', 's3', 'none'), ('c3-hello', 'c3', 'none'), ('c6-hello', 'c6', 'none'), ('pocket-tank', 's3', 'waveshare-amoled18-v2')]
result = {'baseline_revision': '9fdeb323837d090b038a1036d2f344566546d466', 'candidate': 'integration tree; see source-hashes.json',
          'environment': {'os': platform.system(), 'arch': platform.machine(), 'rust': subprocess.check_output(['rustc', '--version'], text=True).strip()},
          'conditions': 'serial alternating pairs, no imposed load gate; short CPU samples, no isolated speed claim',
          'clock': 'delta RUSAGE_CHILDREN.ru_utime with one subprocess at a time',
          'binary_sha256': {k: sha(v.read_bytes()) for k, v in [('baseline', a.baseline), ('candidate', a.candidate)]}, 'workloads': []}
for name, chip, board in workloads:
    manifest = json.loads((a.assets / (name + '.json')).read_text())
    files = manifest['files']
    args = ['--chip', chip, '--board', board, '--boot', 'rom', '--console', 'uart0', '--no-dump', '--max-seconds', '30', '--flash-mb', str(manifest['flash_mb'])]
    if chip == 's3': args += ['--psram-mb', str(manifest['psram_mb'])]
    inputs = {}
    for flag in ['rom', 'bootloader', 'ptable', 'app']:
        f = a.assets / files[flag]; inputs[files[flag]] = sha(f.read_bytes()); args += ['--' + flag, str(f)]
    for off, rel in manifest.get('flash_at', {}).items():
        f = a.assets / rel; inputs[rel] = sha(f.read_bytes()); args += ['--flash-at', off + '=' + str(f)]
    def run(binary):
        before = resource.getrusage(resource.RUSAGE_CHILDREN).ru_utime
        cp = subprocess.run([str(binary.resolve()), *args], capture_output=True, timeout=240)
        user = resource.getrusage(resource.RUSAGE_CHILDREN).ru_utime - before
        err = cp.stderr.decode(errors='replace')
        if cp.returncode: raise RuntimeError(f'{name}: exit {cp.returncode}; inspect local run, no raw capture retained')
        work = re.search(r'— (.*?) insns in ', err)
        cycles = re.search(r'\((\d+) cycles\)', err)
        if not all([work, cycles]): raise RuntimeError(f'{name}: missing work/time report')
        return {'user_s': user, 'insns': work[1], 'cycles': int(cycles[1]), 'console_sha256': sha(cp.stdout), 'load': list(os.getloadavg())}
    for binary in [a.baseline, a.candidate]: run(binary)
    samples = []
    for pair in range(a.pairs):
        order = [('baseline', a.baseline), ('candidate', a.candidate)]
        if pair % 2: order.reverse()
        row = {'pair': pair + 1, 'order': [key for key, _ in order]}
        for key, binary in order: row[key] = run(binary)
        samples.append(row)
    reference = tuple(samples[0]['baseline'][k] for k in ['insns', 'cycles', 'console_sha256'])
    exact = all(tuple(row[key][k] for k in ['insns', 'cycles', 'console_sha256']) == reference for row in samples for key in ['baseline', 'candidate'])
    medians = {key: statistics.median(row[key]['user_s'] for row in samples) for key in ['baseline', 'candidate']}
    result['workloads'].append({'name': name, 'inputs': inputs, 'arguments': [x.replace(str(a.assets), '${ASSETS}') for x in args], 'samples': samples,
                               'equal_work_and_console': exact, 'median_user_s': medians, 'delta_percent': 100 * (medians['candidate'] / medians['baseline'] - 1)})
    a.output.write_text(json.dumps(result, indent=2) + '\n')
    print(name, 'equal work/output:', exact, 'median user CPU:', medians, flush=True)
    if not exact: raise SystemExit('Work/output mismatch; timings are not a valid throughput comparison.')
