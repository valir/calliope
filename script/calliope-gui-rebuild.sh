#!/bin/bash

# run this script from the top directory

pushd src/calliope-gui
npm run build:app # build the release version and the gui
popd
cargo build # also build the debug version
