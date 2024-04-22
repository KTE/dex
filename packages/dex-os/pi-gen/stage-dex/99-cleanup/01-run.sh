#!/bin/bash -e

on_chroot << EOF

systemctl disable bluetooth

apt purge -y build-essential python3-dev pi-bluetooth manpages manpages-dev
apt autoremove -y --purge
apt clean

EOF

# # FIXME: more to patch in pi-gen/export-image/04-finalise/01-run.sh
# # we need to keep the following files for copyright compliance
# BAK_DIR="${ROOTFS_DIR}/tmp/doc-copyright"
# rsync -RavPh "${ROOTFS_DIR}"/usr/share/doc/*/copyright "${BAK_DIR}/"

# rm -rf "${ROOTFS_DIR}"/var/lib/apt/lists/*
# rm -rf "${ROOTFS_DIR}"/usr/share/{locale,groff,doc,man,man-db}

# # restore the copyright files
# rsync -avPh "${BAK_DIR}/usr/share/" "${ROOTFS_DIR}/usr/share/"
# rm -rf "${BAK_DIR}"


