"""Take one bounded CPU stack sample of the frozen mate solver."""
from pathlib import Path
import hashlib
import json
import subprocess
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
BINARY = Path('/private/tmp/sekirei-issue291-d02/target/release/examples/d02_driver')
REG = HERE / 'd02-registration.json'
RECEIPT = HERE / 'd02-receipt.json'
RAW = ROOT / 'data/runs/issue291-d02-20261116'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    assert REG.is_file()
    assert not RECEIPT.exists() and not RAW.exists()
    registration = json.loads(REG.read_text())
    assert sha(BINARY) == registration['binary_sha256']
    assert sha(HERE / 'd02_driver.rs') == registration['driver_sha256']
    assert sha('/usr/bin/sample') == registration['sample_binary_sha256']
    RAW.mkdir()
    sample_path = RAW / 'sample.txt'
    start = time.monotonic()
    process = subprocess.Popen(
        [str(BINARY), registration['case_sfen']],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True,
    )
    ready = process.stdout.readline().strip()
    assert ready == 'ready', ready
    process.stdin.write('go\n')
    process.stdin.flush()
    sample = None
    output = ''
    error = ''
    status = 'incomplete'
    try:
        sample = subprocess.run(
            ['/usr/bin/sample', str(process.pid), '2', '1', '-file', str(sample_path)],
            capture_output=True, text=True, timeout=8,
        )
        output, error = process.communicate(timeout=max(1, 30 - (time.monotonic() - start)))
        if sample.returncode == 0 and process.returncode == 0 and 'complete calls=100 ' in output:
            status = 'complete'
    finally:
        if process.poll() is None:
            process.kill()
            extra_out, extra_err = process.communicate(timeout=3)
            output += extra_out
            error += extra_err
    sample_size = sample_path.stat().st_size if sample_path.exists() else 0
    receipt = dict(
        registration_sha256=sha(REG), status=status, driver_exit=process.returncode,
        driver_stdout=ready + '\n' + output, driver_stderr=error,
        sample_exit=sample.returncode if sample else None,
        sample_stdout=sample.stdout[:4096] if sample else '',
        sample_stderr=sample.stderr[:4096] if sample else '',
        sample_sha256=sha(sample_path) if sample_path.exists() else None,
        sample_bytes=sample_size, elapsed_s=round(time.monotonic() - start, 3),
    )
    RECEIPT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + '\n')
    assert sample_size <= 32 * 1024 * 1024
    assert status == 'complete', receipt
    print(json.dumps(receipt, ensure_ascii=False))


if __name__ == '__main__':
    main()
