## prepare

Install VirtualBox and Vagrant.

on macOs:

```sh
brew cask install virtualbox vagrant
```

## build

1. create the "builder" VM (Debian 32-bit)

   ```sh
   cd packages/builder-vm
   sh build.sh
   cd -
   ```

2. boot "builder" VM and build Rasp

   ```sh
   vagrant up --no-provision

   # fix
   vagrant ssh 'sudo apt update && sudo apt install bc'

   vagrant provision
   ```
