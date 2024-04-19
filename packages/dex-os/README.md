# dex OS

Raspbian-/Debian-based image for Raspberry Pi.

## building `dex os` image

Install

* [Docker](https://www.docker.com) for the build environment
* `quilt` for managing patches

```sh

## build

Builds a custom Raspbian image, based on `2024-03-12-raspios-bullseye-arm64-lite.img`.

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

## alternatives

* <https://github.com/guysoft/CustomPiOS>
* <https://dietpi.com/>
* <https://www.get-edi.io>

## inspiration

* <https://github.com/mogenson/organelle-m-pi-gen>
