$provision_script = <<~"SHELL"
  echo ">>> Generating rpi image ... $@"

  export IMG_NAME="Dexbian"
  export TARGET_HOSTNAME="dexpi"
  export LOCALE_DEFAULT="en_US.UTF-8"
  export KEYBOARD_KEYMAP="us"
  export GIT_HASH="#{`git rev-parse HEAD`.chomp!}"

  # only export the light image
  export STAGE_LIST="stage0 stage1 stage2"

  export DEBIAN_FRONTEND=noninteractive
  export RPIGEN_DIR="${1:-/home/vagrant/rpi-gen}"
  export APT_PROXY='http://127.0.0.1:3142' 
  

  # Prepare. Copy the repo to another location to run as root
  rsync -a --delete --exclude 'work' --exclude 'deploy' \
        "/vagrant/packages/pi-gen/" "${RPIGEN_DIR}/"

  cd ${RPIGEN_DIR}

  # touch stage{0,1,2}/SKIP

  # Clean previous builds. Start always from scratch (the proxy helps here!)
  sudo umount --recursive work/*/stage*/rootfs/{dev,proc,sys} || true
  # sudo rm -rf work/*

  # Build it again
  sudo --preserve-env=APT_PROXY,IMG_NAME,TARGET_HOSTNAME,LOCALE_DEFAULT,KEYBOARD_KEYMAP,GIT_HASH,STAGE_LIST \
    ./build.sh

  # Copy images back to host
  [ -d deploy ] && cp -vR deploy /vagrant/dist
SHELL


Vagrant.configure("2") do |config|
  #config.ssh.insert_key = false
  #config.vm.synced_folder '.', '/vagrant'
  #config.ssh.username = "vagrant"
  #config.ssh.password = "vagrant"

  config.vm.define "virtualbox" do |virtualbox|
    virtualbox.vm.hostname = "rpi-builder-vm"
    virtualbox.vm.box = "file://packages/builder-vm/builds/buster-10.2_rpibuilder-4_virtualbox.box"

    config.vm.provider :virtualbox do |v|
      v.gui = false
      v.memory = 8192
      v.cpus = 4
      v.customize ["modifyvm", :id, "--natdnshostresolver1", "on"]
      v.customize ["modifyvm", :id, "--ioapic", "on"]
    end
    # config.vm.provision "shell", inline: 'echo DONE'
    config.vm.provision "shell", inline: $provision_script, args: "#{ENV['WORK_DIR']}"
  end
end
