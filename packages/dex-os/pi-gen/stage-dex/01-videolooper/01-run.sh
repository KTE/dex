# shellcheck shell=sh

# install adafruit pi_video_looper using their install script, omitting omxplayer

install -v -d -o 1000 -g 1000 files/pi_video_looper "/home/${FIRST_USER_NAME}/pi_video_looper"

on_chroot << EOF

# rm -rf "/home/${FIRST_USER_NAME}/pi_video_looper"
# git clone "https://github.com/adafruit/pi_video_looper" "/home/${FIRST_USER_NAME}/pi_video_looper"

cd "/home/${FIRST_USER_NAME}/pi_video_looper"

# remove 'omxplayer' from the list of packages to install (we only want hello_video)
sed -i 's/omxplayer//g' ./install.sh

./install.sh

EOF

# add our custom video_looper.ini configuration file
install -m 644 files/video_looper.ini "${ROOTFS_DIR}/boot/video_looper.ini"
