#!/bin/bash

# this script should be run from the top directory

cargo build --release -p calliope-stems
install -m755 target/release/calliope-stems ~/.local/bin/
systemctl --user restart calliope-stems
journalctl --user -u calliope-stems -n 5         # shows "listening"

