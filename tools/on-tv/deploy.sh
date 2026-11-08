#!/usr/bin/env bash
# Push the armv7 smoke-test binaries to the TV and run the daemon.
#
# Usage: TV_SSH=root@tv.local tools/on-tv/deploy.sh
# Requires: cross-built binaries in target/armv7-unknown-linux-musleabi/release/
set -euo pipefail

TV_SSH="${TV_SSH:?set TV_SSH=root@tv.local}"
BIN=target/armv7-unknown-linux-musleabi/release
REMOTE_DIR=/tmp/bluepad

cd "$(dirname "$0")/../.."

for bin in bluepad-daemon bluepad-ui; do
    [ -x "$BIN/$bin" ] || {
        echo "missing $BIN/$bin — build first:" >&2
        echo "  cross build --release --target armv7-unknown-linux-musleabi -p bluepad-daemon -p bluepad-ui" >&2
        exit 1
    }
done

echo ">> pushing binaries to $TV_SSH:$REMOTE_DIR"
ssh "$TV_SSH" "mkdir -p $REMOTE_DIR"
scp -q "$BIN/bluepad-daemon" "$BIN/bluepad-ui" "$TV_SSH:$REMOTE_DIR/"

# Run the remote part as a script via stdin: no nested-quote escaping, and
# nothing depends on busybox extras like `timeout`.
echo ">> starting the daemon (root) on the TV"
ssh "$TV_SSH" bash -s "$REMOTE_DIR" <<'REMOTE'
set -u
DIR="$1"
killall bluepad-daemon 2>/dev/null || true
sleep 1
cd "$DIR"
BLUEPAD_DATA_DIR="$DIR" BLUEPAD_SOCKET="$DIR/bluepad.sock" \
    nohup ./bluepad-daemon >stdout.log 2>&1 &
sleep 2
echo '--- daemon log ---'
cat bluepad.log 2>/dev/null || cat stdout.log
echo '--- state view ---'
BLUEPAD_SOCKET="$DIR/bluepad.sock" ./bluepad-ui || true
echo
echo "daemon left running; poke it with:"
echo "  BLUEPAD_SOCKET=$DIR/bluepad.sock $DIR/bluepad-ui <state|watch|pair|cancel|forget|rumble|diag>"
echo "stop it with: killall bluepad-daemon"
REMOTE
