# deadman

USB-based dead man's switch for Linux.

## install

```nix
{
  inputs.deadman.url = "github:dominicegginton/deadman";

  outputs = { deadman, ... }: {
    nixosConfigurations.hostname = {
      modules = [
        deadman.nixosModules.default
        { programs.deadman.enable = true; }
      ];
    };
  };
}
```

## usage

```sh
deadman                      # list devices
sudo deadman tether 1 5      # tether device
sudo deadman status          # check status
sudo deadman severe          # clear tethers
deadman-gui                  # launch gui
```
