# `dex` project

packages:

* [dex OS](./packages/dex-os/README.md)
* [example content](./packages/example-content/README.md)
* [branding](./packages/branding/README.md)

## getting started

Build a dex player by flashing the image to an SD card,
using the official [Raspberry Pi Imager](https://www.raspberrypi.org/software/).

Then boot the Raspberry Pi with the SD card.
It it worked, it will show a 2-second demo video loop.

### advanced

For customizing the player/operating system,
ssh access needs to be enabled.

This can be done using the "customization" feature of the Raspberry Pi Imager,
choosing "Enable SSH" in the "Advanced Options". It is recommended to use key-based authentication.
The user name in the image is `dex` should not be changed, the default password is also `dex` and should be changed if SSH login is enabled and password authentication is used.
  
Then, after booting the Raspberry Pi, ssh into it:
  
```sh
ssh dex@dexpi # or another hostname if you changed it in the customization
```

## development

### creating patches

Upstream repos like `pi-gen` are not forked directly,
rather a series of patches is maintained.
This makes the list of changes we make self-documenting,
and over time should be easier than maintaining a regular fork using `git`.

Good tutorials on using `quilt`:

* <https://raphaelhertzog.com/2012/08/08/how-to-use-quilt-to-manage-patches-in-debian-packages/>
* <https://wiki.debian.org/UsingQuilt>

```sh
quilt new "99-name-of-my-patch"
quilt add ./packages/some-upstream-code/some-file
# edit ./packages/some-upstream-code/some-file
quilt refresh # patchfile is added to ./patches and patch name is added to ./patches/series 
quilt rename "99-better-name-of-my-patch"
```

editing existing patches:

```sh
PATCH_NAME="project/99-name-of-my-patch"
quilt add -P "$PATCH_NAME" ./packages/some-upstream-code/some-file
# edit ./packages/some-upstream-code/some-file
quilt refresh "$PATCH_NAME" # patchfile is updated in ./patches
```

## housekeeping

### update pi-gen repo

```sh
cd packages/pi-gen
git remote add upstream https://github.com/RPi-Distro/pi-gen
git fetch upstream
git push --mirror origin
```
