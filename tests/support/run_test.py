#!/usr/bin/env python3
"""Cargo runner: establish immutable in-process test environment before launch."""
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from resources import resources, process


def main():
    with resources() as (active, diagnostic, env):
        print(f'test diagnostics: {diagnostic}', file=sys.stderr, flush=True)
        external_cwd = env.pop('ANTISLOP_TEST_EXTERNAL_CWD', None)
        if external_cwd not in (None, '1'):
            raise ValueError('ANTISLOP_TEST_EXTERNAL_CWD accepts only 1 or unset')
        cwd = active if external_cwd == '1' else None
        with (diagnostic / 'command.log').open('w') as record:
            record.write(repr(sys.argv[1:]) + f'\ncwd={cwd or Path.cwd()}\n')
        with (diagnostic / 'output.log').open('wb') as log:
            with process(sys.argv[1:], env=env, cwd=cwd, stdout=-1, stderr=-2) as child:
                while chunk := child.stdout.read1(65536):
                    log.write(chunk)
                    log.flush()
                    sys.stdout.buffer.write(chunk)
                    sys.stdout.buffer.flush()
                status = child.wait()
        with (diagnostic / 'command.log').open('a') as record:
            record.write(f'exit={status}\n')
        return status if status >= 0 else 128 - status


if __name__ == '__main__':
    sys.exit(main())
