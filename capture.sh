mkdir -p /tmp/bluepad-probes
while true; do
  {
    echo "=== $(date +%s)"
    ls -l /dev/input/event* 2>/dev/null
    for n in /dev/input/event*; do
      [ -c "$n" ] && echo "LIVE $n" || echo "DEAD $n"
    done
    luna-send -n 1 luna://com.webos.service.bluetooth2/device/getStatus '{}' 2>/dev/null
  } >> /tmp/bluepad-probes/capture.log 2>&1
  sleep 5
done