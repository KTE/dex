on_chroot << EOF

rm -rf "/home/${FIRST_USER_NAME}/pi_video_looper"
git clone "https://github.com/adafruit/pi_video_looper" "/home/${FIRST_USER_NAME}/pi_video_looper"
ls -la "/home/${FIRST_USER_NAME}/pi_video_looper"


set -x

whoami 

ls -la "/home/${FIRST_USER_NAME}"

cd "/home/${FIRST_USER_NAME}/pi_video_looper"

# remove 'omxplayer' from the list of packages to install (we only want hello_video)
sed -i 's/omxplayer//g' ./install.sh

./install.sh

EOF

