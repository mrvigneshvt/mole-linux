#!/usr/bin/env bash
# any.sh <max-seconds> <ws>...  → prints first "<ws> <state>" that settles (idle|blocked), or TIMEOUT
end=$((SECONDS+$1)); shift
while [ $SECONDS -lt $end ]; do
  for w in "$@"; do
    s=$($(dirname "$0")/wait.sh "$w" 20)
    if [ "$s" = idle ] || [ "$s" = blocked ]; then echo "$w $s"; exit 0; fi
  done
done
echo TIMEOUT
