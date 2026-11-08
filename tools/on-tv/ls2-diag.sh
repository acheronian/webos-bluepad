#!/usr/bin/env bash
# Read-only LS2 diagnostics: pin down why `luna-send -n 1` one-shots
# returned EMPTY in the pair-probe run (pair-probe-1794162587.log) while
# the backgrounded `luna-send -i` subscription worked.
#
# The probe's one-shots inherited the script pipe as stdin (ssh bash -s);
# its one working call was backgrounded — which gives stdin /dev/null in a
# non-interactive shell. This script runs under the same transport and
# varies exactly one thing per call: stdin, flags, payload, binary.
#
# Usage: TV_SSH=root@tv.local tools/on-tv/ls2-diag.sh
# Read-only: device/getStatus only; no daemon is stopped, nothing pairs.

TV_SSH="${TV_SSH:?set TV_SSH=root@tv.local}"

cd "$(dirname "$0")/../.."
exec ssh "$TV_SSH" bash -s <<'REMOTE'
URI=luna://com.webos.service.bluetooth2

run() {  # run <label> <cmd...> — prints reply (truncated), exit code
  label="$1"; shift
  echo "--- $label"
  out=$("$@" 2>&1); rc=$?
  printf '%s\n' "$out" | head -c 300
  echo
  echo "    [exit=$rc]"
  echo
}

echo "== luna-send flags =="
luna-send -h 2>&1 | head -20
echo

echo "== hub sockets =="
ls /tmp 2>/dev/null | grep -i -E 'luna|ls2|hub' || echo "(none in /tmp)"
echo

echo "== one-shot device/getStatus: stdin variants =="
run "plain   (stdin = script pipe — the probe's condition)" \
    luna-send -n 1 "$URI/device/getStatus" '{}'
run "</dev/null (the daemon's condition)" \
    luna-send -n 1 "$URI/device/getStatus" '{}' </dev/null
echo 'garbage' | run "stdin = pipe with data" \
    luna-send -n 1 "$URI/device/getStatus" '{}'
run "plain, twice in a row (2nd call)" \
    luna-send -n 1 "$URI/device/getStatus" '{}'
echo

echo "== one-shot: payload and binary variants (all </dev/null) =="
run "subscribe:true payload, one-shot" \
    luna-send -n 1 "$URI/device/getStatus" '{"subscribe":true}' </dev/null
run "luna-send-pub one-shot" \
    luna-send-pub -n 1 "$URI/device/getStatus" '{}' </dev/null
run "different method (adapter/getStatus) — is it device/getStatus-specific?" \
    luna-send -n 1 "$URI/adapter/getStatus" '{}' </dev/null
echo

echo "== control: -i subscription (known to work in the probe run) =="
SUB=/tmp/bluepad-probes/diag-sub.$$.log
mkdir -p /tmp/bluepad-probes
luna-send -i "$URI/device/getStatus" '{"subscribe":true}' >"$SUB" 2>&1 &
pid=$!
sleep 3
kill $pid 2>/dev/null
echo "--- -i reply (first 300 bytes):"
head -c 300 "$SUB"
echo
echo "    [lines=$(wc -l <"$SUB")]"
echo
echo "== done =="
REMOTE
