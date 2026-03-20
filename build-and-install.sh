#!/bin/sh

export HIP_PATH=/opt/rocm 
export ROCM_PATH=/opt/rocm 
export CMAKE_PREFIX_PATH="/opt/rocm;/opt/rocm/lib/cmake"

cargo build --release -p vox-daemon --features "pw,whisper,hipblas,gtk,ui"

sudo install -Dm755 target/release/vox-daemon /usr/local/bin/vox-daemon
