# `dex` project

packages:

* [branding](./packages/branding/README.md)
* [example content](./packages/example-content/README.md)

## building `dex os` image

Install [Docker](https://www.docker.com).

## build

Builds a custom Raspbian image, based on `2024-03-12-raspios-bullseye-arm64-lite.img`.

1. set up customized pi-gen

   ```sh
   git submodule update --recursive --init --force packages/pi-gen
   quilt pop -af
   quilt push -a
   ```

1. build Raspbian-based image in docker container

   ```sh
   time ./build.sh
   ```

## roadmap

1. add customizations (stage )

   * more packages: omxplayer

2. more overriding. one option how to do this:
   * custom stage2 with less stuff and our customizations
     * exports the "PROD" image
   * custom stage3 with more developer tools
     * exports the "DEV" image
   * alternative: stage 3 does more removals and cleanup, then exports the "DEV" image. a bit less clean but maybe simpler to handle the forked code/config.

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
quilt refresh # pathfile is added to ./patches and name added to ./patches/series 
quilt rename "99-better-name-of-my-patch"
```

## alternatives

* <https://github.com/guysoft/CustomPiOS>
* <https://dietpi.com/>
* <https://www.get-edi.io>

## inspiration

* <https://github.com/mogenson/organelle-m-pi-gen>

## housekeeping

### update pi-gen repo

```sh
cd packages/pi-gen
git remote add upstream https://github.com/RPi-Distro/pi-gen
git fetch upstream
git push --mirror origin
```
