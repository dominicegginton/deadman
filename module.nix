{ lib
, config
, pkgs
, ...
}:

{
  options.programs.deadman = {
    enable = lib.mkEnableOption "Deadman";

    package = lib.mkPackageOption pkgs "deadman" { };
  };

  config = lib.mkIf config.programs.deadman.enable {
    environment.systemPackages = [ config.programs.deadman.package ];

    environment.pathsToLink = [ "/share/applications" "/share/icons" ];

    systemd.packages = [ config.programs.deadman.package ];

    systemd.services.deadmand = {
      description = "Deadman daemon";
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        ExecStart = "${config.programs.deadman.package}/bin/deadmand";
        Restart = "on-failure";
        RestartSec = 5;
      };
    };
  };
}
