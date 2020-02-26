## prepare

Install VirtualBox and Vagrant.

on macOs:

```sh
brew install  packer jq quilt
brew cask install virtualbox vagrant
```

## build

1. create the "builder" VM (Debian 32-bit)

   ```sh
   cd packages/builder-vm
   sh build.sh
   cd -
   ```

4. set up customized pi-gen

   ```sh
   git submodule update --recursive --init --force packages/pi-gen
   quilt pop -af
   quilt push -a
   ```

3. boot "builder" VM and build Raspbian-based image

   ```sh
   vagrant up --no-provision # start VM
   vagrant provision         # run build script from clean state
   ```

## development

### creating patches

upstream repos like `pi-gen` are not forked directly,
rather a series of patches is maintained.
this makes the list of changes we make self-documenting,
and over time should be easier than maintaining a regular fork using `git`.

the *result* of applying the pathes are then checked in,
meaning the patches are only used when developing in the main repo,
not when using it to build images.
this could be automated:every push to `master` of repo with patches
is applied to the forked repo (using the referenced version of submodule).

```sh
cd dex-os
quilt new "99-name-of-my-patch"
quilt add ./packages/some-upstream-code/some-file
# edit ./packages/some-upstream-code/some-file
quilt refresh # pathfile is added to ./patches and name added to ./patches/series 
quilt rename "99-better-name-of-my-patch"
```


   # fix
   vagrant ssh 'sudo apt update && sudo apt install bc'

   vagrant provision
   ```
