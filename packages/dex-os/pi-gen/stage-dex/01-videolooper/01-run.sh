# shellcheck shell=sh

# install adafruit pi_video_looper using their install script, omitting omxplayer

rm -rf "/home/${FIRST_USER_NAME}/pi_video_looper"
cp -r files/pi_video_looper "${ROOTFS_DIR}/home/${FIRST_USER_NAME}/"

on_chroot << EOF

cd "/home/${FIRST_USER_NAME}/pi_video_looper"

# remove 'omxplayer' from the list of packages to install (we only want hello_video)
sed -i 's/omxplayer//g' ./install.sh

./install.sh

EOF

# add our custom video_looper.ini configuration file
install -m 644 files/video_looper.ini "${ROOTFS_DIR}/boot/video_looper.ini"
