#!/bin/sh
# A stand-in for an OLDER UberWave (no IPC, no --version) in GUI scripts
# (CDEPT_UBERWAVE=<this file>): records the arguments, one per line, in
# uberwave-args.txt of the folder it is started in (the run point's), and the folder
# itself in uberwave-cwd.txt. See uberwave.script. fake-uberwave-ipc.py is the one
# with IPC.
case "$1" in --version) exit 0 ;; esac
printf '%s\n' "$@" > uberwave-args.txt
pwd > uberwave-cwd.txt
