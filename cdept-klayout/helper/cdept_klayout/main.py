# cdept-klayout: start-up script, run by KLayout itself.
# SPDX-License-Identifier: GPL-3.0-or-later
#
#   QT_QPA_PLATFORM=offscreen klayout -z -nc -e -r .../cdept_klayout/main.py \
#       -rd connect=<socket> [-rd frame_dir=<folder>] [-rd display=1]
#
# `-rd name=value` arrive as global variables of this script.

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from cdept_klayout import server  # noqa: E402

status = server.run(globals())
sys.stdout.flush()
sys.stderr.flush()
# Leave without Qt's teardown: nothing is left to save, and it can hang offscreen.
os._exit(status or 0)
