"""Preserve O01's failed preflight and prepare a fresh isolated R2 build."""
from pathlib import Path
import prepare_o01 as candidate

candidate.PLAIN = Path('/private/tmp/sekirei-issue291-o01-r2')
candidate.COUNTED = Path('/private/tmp/sekirei-issue291-o01-r2-counted')
candidate.PATCH = candidate.HERE / 'o01-r2.patch'

if __name__ == '__main__':
    candidate.main()
