#!/bin/bash -e

on_chroot << EOF

systemctl disable bluetooth

EOF

apt purge -y build-essential python3-dev pi-bluetooth manpages manpages-dev
apt autoremove -y --purge
apt clean

# we need to keep the following files for copyright compliance
BAK_DIR="$(mktemp -d)"
rsync -RavPh /usr/share/doc/*/copyright "${BAK_DIR}/"

rm -rf "${ROOTFS_DIR}/var/lib/apt/lists/"*
rm -rf "${ROOTFS_DIR}/usr/share/"{locale,groff,doc,man,man-db}

# restore the copyright files
rsync -avPh "${BAK_DIR}/usr/share/doc/" /usr/share/doc/
