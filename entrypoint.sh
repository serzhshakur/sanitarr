#!/bin/bash

# the interval at which the sanitarr binary will be executed
INTERVAL=${INTERVAL:-1h}

while true; do
  /app/sanitarr ${@}
  sleep $INTERVAL
done
