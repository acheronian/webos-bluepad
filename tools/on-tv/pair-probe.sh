#!/usr/bin/env bash
# Raw-surface pairing probe: exercise the exact bluetooth2 call sequence
# the daemon's stock path uses (crates/bluepad-radio/src/ls2.rs), with
# full logging, so ONE run on the TV settles every remaining on-device
# pairing unknown (#32):
#
#   1. luna-send -i subscribe mode works at all (the probe harness only
#      ever proved -n 1 one-shots; the daemon's pair flow AND pairable
#      gate both depend on -i)
#   2. the pairable gate (adapter/awaitPairingRequests)
#   3. discovery candidates via device/getStatus (named, unpaired,
#      peripheral-class — the daemon's candidate filter)
#   4. the adapter/pair subscription event stream (displayPinCode /
#      displayPasskey / confirmPassKey / endPairing)
#   5. hid/connect with real error codes, then hid/disconnect and
#      adapter/unpair teardown specimens (#36's calls)
#   6. whether luna-send-pub exists (the last open #32 item)
#
# Usage: TV_SSH=root@tv.local tools/on-tv/pair-probe.sh [MAC]
# Put the pad in pairing mode first. Without a MAC, the first unpaired
# named peripheral-class device seen during the 20 s discovery window
# is picked automatically.
#
# Leaves the TV clean: after a successful hid/connect the pad is
# disconnected and unpaired. If hid/connect fails, the pad stays PAIRED
# for inspection (unpair command printed). A running bluepad-daemon is
# stopped first — the probe needs exclusive pairing; restart it with
# tools/on-tv/deploy.sh afterwards.
#
# All output is teed to /tmp/bluepad-probes/pair-probe-<epoch>.log on
# the TV — copy it back as a specimen (the probe-logs/ pattern).

TV_SSH="${TV_SSH:?set TV_SSH=root@tv.local}"

cd "$(dirname "$0")/../.."
exec ssh "$TV_SSH" bash -s -- "$@" <<'REMOTE'
set -u
URI=luna://com.webos.service.bluetooth2
PROBE_DIR=/tmp/bluepad-probes
mkdir -p "$PROBE_DIR"
LOG="$PROBE_DIR/pair-probe-$(date +%s).log"

log() { echo "[$(date +%s)] $*" | tee -a "$LOG"; }
# luna-send exits SILENTLY (zero bytes, exit 0) when stdin is an open pipe —
# captured on the TV: every foreground one-shot over the ssh script pipe
# returned nothing while the backgrounded -i call (/dev/null stdin) worked;
# ls2-diag.sh confirmed /dev/null stdin replies fine. Pin stdin everywhere.
luna() { luna-send "$@" </dev/null; }
raw() { luna "$@" 2>&1 | tee -a "$LOG"; }

GATE_PID= PAIR_PID= SUB_PID= ADDR=${1:-}
cleanup() {
  log "--- cleanup ---"
  if [ -n "$ADDR" ]; then
    luna -n 1 "$URI/adapter/cancelPairing" "{\"address\":\"$ADDR\"}" >>"$LOG" 2>&1
  fi
  luna -n 1 "$URI/adapter/cancelDiscovery" '{}' >>"$LOG" 2>&1
  kill $GATE_PID $PAIR_PID $SUB_PID 2>/dev/null
  log "log: $LOG"
  log "if the pairable gate wedges later (errorCode 104), run: killall luna-send"
}
trap cleanup EXIT

log "=== bluepad pairing probe ==="

# --- 0. preflight ---
if killall bluepad-daemon 2>/dev/null; then
  log "stopped a running bluepad-daemon (exclusive pairing; restart with deploy.sh)"
fi
killall luna-send 2>/dev/null  # release orphaned -i subscribers holding gate slots
command -v luna-send >/dev/null 2>&1 || { log "FATAL: no luna-send on PATH"; exit 1; }
for b in luna-send-pub luna-send; do
  command -v "$b" >/dev/null 2>&1 && log "present: $b -> $(command -v "$b")"
done
ls /usr/bin/luna-send* /usr/sbin/luna-send* /bin/luna-send* 2>/dev/null | tee -a "$LOG"
log "--- baseline device/getStatus ---"
raw -n 1 "$URI/device/getStatus" '{}'

# --- 1. subscribe mode: the load-bearing unknown ---
log "--- subscribe-mode check: luna-send -i device/getStatus (4 s) ---"
SUB_OUT="$PROBE_DIR/sub-check-$$.log"
luna -i "$URI/device/getStatus" '{"subscribe":true}' >"$SUB_OUT" 2>&1 &
SUB_PID=$!
sleep 4
kill $SUB_PID 2>/dev/null
SUB_PID=
cat "$SUB_OUT" >>"$LOG"
n=$(grep -c returnValue "$SUB_OUT" 2>/dev/null)
n=${n:-0}
if [ "$n" -ge 1 ]; then
  log "PASS: luna-send -i streams subscriptions ($n response line(s)) — the daemon's pair flow and gate can work"
else
  log "FAIL: luna-send -i produced no subscription responses — the daemon's pair flow and gate CANNOT work as written; specimen: $SUB_OUT"
  exit 1
fi

# --- 2. pairable gate ---
log "--- opening the pairable gate: adapter/awaitPairingRequests ---"
GATE_OUT="$PROBE_DIR/gate-$$.log"
luna -i "$URI/adapter/awaitPairingRequests" '{"subscribe":true}' >"$GATE_OUT" 2>&1 &
GATE_PID=$!
sleep 1
if grep -q errorCode "$GATE_OUT" 2>/dev/null; then
  log "gate refused — another subscriber may hold the slot (errorCode 104):"
  cat "$GATE_OUT" | tee -a "$LOG"
fi

# --- 3. discovery ---
log "--- adapter/startDiscovery (20 s window; put the pad in pairing mode now) ---"
raw -n 1 "$URI/adapter/startDiscovery" '{}'
PICK=
if [ -n "$ADDR" ]; then
  log "target given: $ADDR — brief discovery so the stack sees it, then pair"
  sleep 4
else
  deadline=$(($(date +%s) + 20))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    status=$(luna -n 1 "$URI/device/getStatus" '{}' 2>/dev/null)
    echo "$status" >>"$LOG"
    # candidate filter, mirroring ls2.rs's is_pairing_candidate: unpaired,
    # named, peripheral major class (classOfDevice & 0x1F00 == 0x0500).
    # Devices are split on the "},{" separator; classOfDevice sits after
    # the nested manufacturerData{}, so splitting on "}" would lose it.
    PICK=$(printf '%s\n' "$status" | sed 's/},{/}\n{/g' | while read -r chunk; do
      case "$chunk" in
        *'"paired":false'*)
          name=$(printf '%s' "$chunk" | sed -n 's/.*"name":"\([^"]*\)".*/\1/p')
          addr=$(printf '%s' "$chunk" | sed -n 's/.*"address":"\([^"]*\)".*/\1/p')
          cod=$(printf '%s' "$chunk" | sed -n 's/.*"classOfDevice":\([0-9]*\).*/\1/p')
          [ -n "$name" ] && [ -n "$addr" ] || continue
          [ $(( (cod & 0x1F00) == 0x0500 ? 1 : 0 )) -eq 1 ] || continue
          echo "$addr"
          ;;
      esac
    done | head -1)
    if [ -n "$PICK" ]; then
      log "discovered candidate: $PICK"
      break
    fi
    sleep 2
  done
  if [ -z "$PICK" ]; then
    log "FAIL: DiscoveryTimeout — no unpaired named peripheral-class device in 20 s"
    exit 1
  fi
  ADDR=$PICK
fi
luna -n 1 "$URI/adapter/cancelDiscovery" '{}' >>"$LOG" 2>&1
log "pairing target: $ADDR"

# --- 4. adapter/pair subscription (30 s bound, the spec's pair window) ---
paired_ok=
if luna -n 1 "$URI/device/getStatus" '{}' 2>/dev/null | grep -q "\"address\":\"$ADDR\",\"paired\":true"; then
  log "already paired (incoming flow may have completed) — skipping adapter/pair"
  paired_ok=1
fi
if [ -z "$paired_ok" ]; then
  log "--- adapter/pair subscription (30 s bound) ---"
  PAIR_OUT="$PROBE_DIR/pair-events-$$.log"
  : >"$PAIR_OUT"
  luna -i "$URI/adapter/pair" "{\"address\":\"$ADDR\",\"subscribe\":true}" >"$PAIR_OUT" 2>&1 &
  PAIR_PID=$!
  seen=0
  deadline=$(($(date +%s) + 30))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    total=$(wc -l <"$PAIR_OUT")
    if [ "$total" -gt "$seen" ]; then
      tail -n +"$((seen + 1))" "$PAIR_OUT" | tee -a "$LOG"
      seen=$total
    fi
    if grep -q '"request":"endPairing"' "$PAIR_OUT" 2>/dev/null; then
      # success = endPairing carrying returnValue:true (legacy DeviceFinder)
      if grep '"request":"endPairing"' "$PAIR_OUT" | tail -1 | grep -q '"returnValue":true'; then
        paired_ok=1
      fi
      break
    fi
    # incoming flow: paired flips true in device/getStatus without any
    # outgoing endPairing event (mirrors the daemon's dual-flow completion)
    if luna -n 1 "$URI/device/getStatus" '{}' 2>/dev/null | grep -q "\"address\":\"$ADDR\",\"paired\":true"; then
      log "paired via the incoming flow (paired flipped true, no endPairing event)"
      paired_ok=1
      break
    fi
    sleep 2
  done
  kill $PAIR_PID 2>/dev/null
  PAIR_PID=
  # a killed luna-send orphans the subscription; cancelPairing (cleanup
  # trap) releases the active pairing request so future attempts work
fi
if [ -n "$paired_ok" ]; then
  log "PAIR SUCCESS"
else
  log "FAIL: pair window closed without completion (see events above)"
  exit 1
fi

# --- 5. hid/connect ---
log "--- hid/connect ---"
reply=$(raw -n 1 "$URI/hid/connect" "{\"address\":\"$ADDR\"}")
case "$reply" in
  *'"returnValue":true'*)
    log "CONNECT SUCCESS"
    ;;
  *)
    code=$(printf '%s' "$reply" | sed -n 's/.*"errorCode":\([0-9]*\).*/\1/p')
    log "CONNECT FAILED (errorCode=${code:-?}: 106 unreachable, 129 auth, 130 stack died mid-handshake)"
    log "pad left PAIRED for inspection; clean up with:"
    log "  luna-send -n 1 $URI/adapter/unpair '{\"address\":\"$ADDR\"}'"
    exit 1
    ;;
esac

# --- 6. teardown specimens: hid/disconnect + adapter/unpair (#36's calls) ---
sleep 2
log "--- hid/disconnect (teardown specimen) ---"
raw -n 1 "$URI/hid/disconnect" "{\"address\":\"$ADDR\"}"
log "--- adapter/unpair (leaves the TV clean) ---"
raw -n 1 "$URI/adapter/unpair" "{\"address\":\"$ADDR\"}"
log "=== probe complete ==="
REMOTE
