#!/bin/sh
# Replay the pdk-setup tutorial on the real GUI (needs a display and the IHP PDK).
#
#   cdept-gui/scripts/tutorial.sh [FOLDER-FOR-SCREENSHOTS]
#
# PDKS is the folder that holds ihp-sg13g2 (default /opt/ext/OpenPDKs). PDK_ROOT is
# left unset: the tutorial must work for someone who has not set it. HOME and the
# settings are scratch folders.
set -e
here=$(cd "$(dirname "$0")" && pwd)
cdept=${CDEPT:-$here/../../target/debug/cdept}
pdks=${PDKS:-/opt/ext/OpenPDKs}
work=${1:-$(mktemp -d)}
mkdir -p "$work/home" "$work/data" "$work/cwd"
rm -rf "$work/home/cdept-tutorial"
sed -e "s|@SCRATCH@|$work|g" -e "s|@PDKS@|$pdks|g" \
  "$here/tutorial-pdk-setup.script" > "$work/tutorial.script"
cat > "$work/start.script" <<END
guide start pdk-setup
END
cat "$work/tutorial.script" >> "$work/start.script"
cd "$work/cwd"
env -u PDK_ROOT -u PDK HOME="$work/home" XDG_DATA_HOME="$work/data" \
  "$cdept" gui --script "$work/start.script"
project=$work/home/cdept-tutorial/my-first-chip
ls "$project"
# The commands the last step of the tutorial gives.
for command in info "tech notes"; do
  env -u PDK_ROOT -u PDK HOME="$work/home" XDG_DATA_HOME="$work/data" \
    "$cdept" -p "$project" $command
done
echo "screenshots in $work"
