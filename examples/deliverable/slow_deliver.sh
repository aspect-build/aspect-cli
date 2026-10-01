#!/usr/bin/env bash
# Outlives several of the delivery task's update intervals, then fails unless
# the task kept emitting updates while it ran: those updates carry the GitHub
# check-run heartbeat. //.aspect/config.axl stamps each update's epoch second
# into $ASPECT_TEST_TASK_UPDATE_STAMP.
set -euo pipefail
stamp="${ASPECT_TEST_TASK_UPDATE_STAMP:?set ASPECT_TEST_TASK_UPDATE_STAMP to a writable file path}"
start="$(date +%s)"
sleep 15
last="$(cat "$stamp")"
if ((last < start + 5)); then
    echo "delivery task sent no update while this target ran: last update at ${last}, target started at ${start}" >&2
    exit 1
fi
echo "delivery task kept updating while this target ran (last update $((last - start))s after start)"
