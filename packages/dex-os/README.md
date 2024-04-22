# dex OS

Raspbian-/Debian-based image for Raspberry Pi.

## building `dex os` image

Install

* [Docker](https://www.docker.com) for the build environment
* `quilt` for managing patches

```sh

## build

Builds a custom Raspbian/Raspberry Pi OS image, based on Debian `buster`.

1. set up customized pi-gen

    ```sh
    ./prepare.sh
    ```

1. build in docker container

   ```sh
   time ./build.sh
   ```

## development

for incremental build, first run:

```sh
time PRESERVE_CONTAINER=1 CONTINUE=0 ./build.sh
```

and then 

```sh
time PRESERVE_CONTAINER=1 CONTINUE=1 ./build.sh
```

debug the container:

```sh
./dev-debug-container.sh
# gives root shell in the build container
cd pi-gen/work/…
# for example, delete our stage for faster rebuild
rm -rf stage-dex
```

starting from scratch:

```sh
./dev-reset.sh
```

## notes

### data partition

The image has a third partition, formatted as FAT, for storing media and configuration files.
Its mounted under `/dexdata` on the Raspberry Pi.
It can be managed by mounting the SD card on a computer (macOS, Windows, Linux) and copying files to it.

Creating and handling this partition differs from the default Raspberry Pi OS behavior:
* Raspbian/Raspberry Pi OS uses a single partition `rootfs` partition for the operating system and user data.
  * On first boot, the `rootfs` *partition* is expanded to fill the SD card.
    * This is done by settings the `init` (in `/boot/cmdline.txt`) to `init=/usr/lib/raspi-config/init_resize.sh`
    * After this script ran, it removes the `init` line from `/boot/cmdline.txt` so the system will boot regularly from then on.
    * Script source: <https://github.com/RPi-Distro/raspi-config/blob/de70c08c7629b2370d683193a62587ca30051e36/usr/lib/raspi-config/init_resize.sh>
  * On the second boot, the `rootfs` *filesystem* is resized to fill the now expanded partition.
    * This is done by the `resize2fs_once` script in `/etc/init.d`
    * This script will also remove itself after its done, so it only runs once.
    * There is not reboot necessary after this step (it runs early enough in the boot process).
  * Note: those steps are necessary because modifying the `rootfs` partition, where the operating system is running from, is not possible while it is mounted. 

* dexOS handles it differently:
  * The `rootfs` partition is not expanded on first boot (the special `init` is already removed `/boot/cmdline.txt`).
  * The `resize2fs_once` script therefore runs on first boot, and is customized to handle the data partition only
    * It resizes the `dexdata` *partition* to fill the whole SD card.
    * It *recreates* the `dexdata` *filesystem* to fill the partition.
      * This is necessary because the `resize2fs` command not resize FAT partitions, and `fatresize` did not work reliably.
      * The data in this partition is kept by backing it up to the `rootfs`, which means it must be rather small initially so there is enough space (currently 10MB).
  * NOTE: right now this is a compromise. After flashing the image, it could immediately be mounted on the same computer, so the media can be copied directly to the SD card.
    However, 10MB is only enough for the very small example video. 
      * Only after booting the SD card with an Raspberry Pi, the data partition will be expanded to the full size of the SD card.
        * That means the data partition could also be ignored in the build step and only be created on first boot, but it already works.
      * On the other hand, a script could be used to resize the partition on the same computer that was used to flash the images, but then its not as easy to use a just using the official Raspberry Pi Imager.

## alternatives

* <https://github.com/guysoft/CustomPiOS>
* <https://dietpi.com/>
* <https://www.get-edi.io>

## inspiration

* <https://github.com/mogenson/organelle-m-pi-gen>
