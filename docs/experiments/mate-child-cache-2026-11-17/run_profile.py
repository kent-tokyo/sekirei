"""Take one bounded two-second CPU sample of current H01 solver."""
from pathlib import Path
import hashlib
import json
import os
import signal
import subprocess
import time
from datetime import datetime, timezone

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[2]
OUT=ROOT/'data/runs/mate-child-cache-20261117/profile'
BUILD=Path('/private/tmp/sekirei-child-cache-20261117-profile')
BINARY=BUILD/'target/release/examples/profile_driver'


def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def main():
    assert not OUT.exists()
    OUT.mkdir()
    sfen=json.loads((ROOT/'data/runs/jsa-tsume-20261116/corpus.json').read_text())['cases'][0]['sfen']
    reg=dict(id='MATE-P01-20261117',registered_utc=datetime.now(timezone.utc).isoformat(),
             source='9f3539f29d6ab85fbe75a733953f271e633eca7f',sfen=sfen,
             hashes={str(p):sha(p) for p in [BINARY,BUILD/'crates/sekirei-core/src/mate.rs',
                     BUILD/'crates/sekirei-core/examples/profile_driver.rs',HERE/'PROFILE_PLAN.md',Path(__file__),Path('/usr/bin/sample')]},
             calls=100,sample_seconds=2,interval_ms=1,outer_seconds=30,rss_kib=2097152)
    (OUT/'registration.json').write_text(json.dumps(reg,indent=2)+'\n')
    start=time.monotonic(); peak_rss=0; status='complete'; sample=None
    with (OUT/'driver.stderr').open('w') as err:
        process=subprocess.Popen([str(BINARY),sfen],stdin=subprocess.PIPE,stdout=subprocess.PIPE,
                                 stderr=err,text=True,start_new_session=True)
        try:
            assert process.stdout.readline().strip()=='ready'
            process.stdin.write('go\n');process.stdin.flush()
            with (OUT/'sample.stdout').open('w') as so,(OUT/'sample.stderr').open('w') as se:
                sample=subprocess.Popen(['/usr/bin/sample',str(process.pid),'2','1','-file',str(OUT/'sample.txt')],stdout=so,stderr=se)
                while process.poll() is None or sample.poll() is None:
                    elapsed=time.monotonic()-start
                    assert elapsed<30,'driver timeout'
                    assert sample.poll() is not None or elapsed<8,'sample timeout'
                    rss=subprocess.run(['ps','-o','rss=','-p',str(process.pid)],capture_output=True,text=True,timeout=2)
                    assert rss.returncode in (0,1)
                    if rss.stdout.strip(): peak_rss=max(peak_rss,int(rss.stdout.strip()))
                    assert peak_rss<=2097152,'rss cap'
                    for p in OUT.iterdir(): assert p.stat().st_size <=32*1024**2,'output cap'
                    time.sleep(.1)
            output=process.stdout.read()
            assert process.wait(timeout=2)==0 and sample.wait(timeout=2)==0
            assert 'complete calls=100' in output
        except Exception as exc:
            status=repr(exc);output=''
        finally:
            if process.poll() is None:
                os.killpg(process.pid,signal.SIGKILL);process.wait(timeout=2)
            if sample is not None and sample.poll() is None:
                sample.kill();sample.wait(timeout=2)
            process.stdin.close();process.stdout.close()
    result=dict(status=status,stdout=output,elapsed_s=time.monotonic()-start,sampled_peak_rss_kib=peak_rss,
                process_reaped=True,sample_exit=sample.returncode if sample else None,
                sample_sha256=sha(OUT/'sample.txt') if (OUT/'sample.txt').exists() else None,
                registration_sha256=sha(OUT/'registration.json'))
    (OUT/'receipt.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))
    assert status=='complete'


if __name__=='__main__':main()
