#!/bin/bash
set -euo pipefail

/usr/local/bin/privilege-honeypot-tests --ignored --exact \
    real_sudo_runs_pacman_query_through_owner_runner --nocapture
