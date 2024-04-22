#!/bin/bash -e

EXAMPLE_FILE="files/dex-test-card-2s-1080p.h264"

mkdir -p "${ROOTFS_DIR}/dexdata/"
install "$EXAMPLE_FILE" "${ROOTFS_DIR}/dexdata/"
