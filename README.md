# `dex` project

packages:

* [dex OS](./packages/dex-os/README.md)
* [example content](./packages/example-content/README.md)
* [branding](./packages/branding/README.md)

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

## housekeeping

### update pi-gen repo

```sh
cd packages/pi-gen
git remote add upstream https://github.com/RPi-Distro/pi-gen
git fetch upstream
git push --mirror origin
```
