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

echo ">> starting the daemon (root) on the TV"
ssh "$TV_SSH" "
    # stop a previous smoke daemon, if any
    killall bluepad-daemon 2>/dev/null || true
    sleep 1
    cd $REMOTE_DIR
    BLUEPAD_DATA_DIR=$REMOTE_DIR BLUEPAD_SOCKET=$REMOTE_DIR/bluepad.sock \
        nohup ./bluepad-daemon >stdout.log 2>&1 &
    sleep 2
    echo '--- daemon log ---'
    cat bluepad.log 2>/dev/null || cat stdout.log
    echo '--- state view ---'
    BLUEPAD_SOCKET=$REMOTE_DIR/bluepad.sock timeout 5 ./bluepad-ui || true
    echo
    echo \"daemon left running; poke it with: BLUEPAD_SOCKET=$REMOTE_DIR/bluepad.sock ./bluepad-ui\"
    echo \"stop it with: ssh $TV_SSH killall bluepad-daemon\"
"
