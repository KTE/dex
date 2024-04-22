# TODO: convert those to proper patches? Makes the repo less readable, but catches unexpected changes from upstream

# disable the default init_resize.sh script (we dont need to resize the root partition)
sed -i 's| init=/usr/lib/raspi-config/init_resize.sh||' "${ROOTFS_DIR}/boot/cmdline.txt"

# install our resize2fs_once script, which resizes the data partition on first boot
install -m 755 files/resize2fs_once "${ROOTFS_DIR}/etc/init.d/"
