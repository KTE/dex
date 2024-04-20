# old notes

## Hardware Interfacing

* <http://www.elinux.org/RPi_Low-level_peripherals>
* <http://log.liminastudio.com/writing/tutorials/tutorial-how-to-use-your-raspberry-pi-like-an-arduino>
* <https://projects.drogon.net/raspberry-pi/wiringpi/>

## Howto from Raspbian

1. Flash SD card
2. Boot raspian
3. `raspi-config` (starts automatically on first boot
    * locale
    * enable ssh
    * reboot
4. ssh pi@\$IP

    ```sh
    # got root?
    sudo -i

    # update debian
    apt-get update && apt-get upgrade
    apt-get install vim tmux build-essential dosfstools omxplayer x11-server-utils

    # ## setup
    #
    # ### hostname
    # - get the serial number of your pi
    cat /proc/cpuinfo
    #… Serial  : 00000000023caffee
    #
    # - set it as hostname postfix, without leading zeros
    node /usr/local/exhd/scripts/set-hostname.js

    vim /etc/default/rcS
    FSCKFIX=yes

    # ### silentboot
    update-rc.d console-setup enable
    update-rc.d plymouth enable
    update-rc.d kbd enable

    vim /etc/inittab
    #1:2345:respawn:/sbin/getty --noclear 38400 tty1
    #…
    #6:

    vim /boot/config.txt # > "disable_splash=1"
    vim /boot/cmdline.txt # > "console=tty2 loglevel=3 quiet"
    vim /etc/sysctl.conf # > "#kernel.printk"
    vim /etc/kbd/config # > "BLANK_TIME=1"

    # ### ssh key
    ssh-keygen # enter-enter-enter
    cat ~/.ssh/id_rsa.pub

    ### dotfiles
    git clone https://github.com/KTE/dotfiles /usr/share/dotfiles
    git clone /usr/share/dotfiles ~/.dotfiles && source ~/.dotfiles/bootstrap.sh -f && cd -

    ## install software
    ### nodejs
    cd /tmp
    NODEV="0.10.22"
    wget "http://nodejs.org/dist/v$NODEV/node-v$NODEV-linux-arm-pi.tar.gz"
    tar -xvzf "node-v$NODEV-linux-arm-pi.tar.gz"
    mkdir -p /usr/local/node
    mv node-v$NODEV-linux-arm-pi/* /usr/local/node/
    ln -s /usr/local/node/bin/{node,npm} /usr/bin/

    # ### omxplayer
    #git clone https://github.com/stewiem2000/omxplayer
    #cd omxplayer/
    #git checkout seamless-looping
    #make ffmpeg && \
    #make && \
    #make dist

    # ### wiring-pi
    cd
    git clone git://git.drogon.net/wiringPi
    cd wiringPi
    ./build

    # install exhd
    git clone …/exhibitor
    cd exhibitor/exhd.js
    npm link
    exhd

    # media partition
    cfdisk /dev/mmcblk0
    # new, w95 fat32

    mkdir /media/EXHIBITOR
    vim /etc/fstab
    #> /dev/mmcblk0p3 /media/EXHIBITOR vfat defaults 0 2

    # files to copy:
    # - /etc/rc.local
    # - exhd ?

    # cleanup & shutdown
    cd
    apt-get clean
    rm -rf /tmp/*
    halt -p # turn off
    ```

## Howto from Arch Linux

1. Flash SD card
2. Boot archlinuxarm
3. ssh root@\$IP

    ```shell
    # update & install
    pacman -Syu git rsync nodejs make gcc python2  omxplayer wiringpi

    # ### ssh key
    ssh-keygen # enter-enter-enter
    cat ~/.ssh/id_rsa.pub

    # ### dotfiles
    git clone https://github.com/KTE/dotfiles /usr/share/dotfiles
    git clone /usr/share/dotfiles ~/.dotfiles && source ~/.dotfiles/bootstrap.sh -f && cd -

    # install exhd
    cd
    git clone https://github.com/fivdi/epoll.git && cd epoll && git checkout v0.1.2
    npm i
    ```

    wget/rsync … /usr/local/exhd
    cd /usr/local/exhd
    npm link
    ```
