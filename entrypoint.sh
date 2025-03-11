#!/bin/bash

# the interval at which the sanitarr binary will be executed
INTERVAL=${INTERVAL:-1h}

while true; do
  sanitarr ${@}
  sleep $INTERVAL
done
