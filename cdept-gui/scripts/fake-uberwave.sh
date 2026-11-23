#!/bin/sh
# A stand-in for UberWave in GUI scripts (CDEPT_UBERWAVE=<this file>): records the
# arguments, one per line, in uberwave-args.txt of the folder it is started in (the
# run point's), and the folder itself in uberwave-cwd.txt. See uberwave.script.
printf '%s\n' "$@" > uberwave-args.txt
pwd > uberwave-cwd.txt
