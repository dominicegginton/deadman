{ lib
, rustPlatform
, makeDesktopItem
, systemd
, rustfmt
, pkg-config
, gcc
, glib
, gtk4
, libadwaita
, cairo
}:

let
  pkg = rustPlatform.buildRustPackage
    rec {
      name = "deadman";
      src = ./.;
      cargoLock.lockFile = ./Cargo.lock;
      runtimeInputs = [ systemd ];
      nativeBuildInputs = [ rustfmt pkg-config gcc glib ];
      buildInputs = [ gtk4 libadwaita cairo ];
      meta = {
        description = "A USB-based deadman's switch for Linux systems";
        homepage = "https://github.com/dominicegginton/deadman";
        platforms = lib.platforms.linux;
      };
    };

  desktopFile = makeDesktopItem {
    name = "dev.dominicegginton.${pkg.name}";
    desktopName = pkg.name;
    comment = "Systemd based usb device deadman kill switch";
    exec = "${pkg}/bin/deadman-gui";
    icon = pkg.name;
    categories = [ "Utility" ];
  };

  desktopIcon = ./deadman-gui/icon.svg;
in

pkg.overrideAttrs (_: {
  postInstall = ''
    mkdir -p $out/share/applications
    mkdir -p $out/share/icons/hicolor/scalable/apps

    cp ${desktopFile}/share/applications/dev.dominicegginton.${pkg.name}.desktop $out/share/applications/dev.dominicegginton.${pkg.name}.desktop
    cp ${desktopIcon} $out/share/icons/hicolor/scalable/apps/dev.dominicegginton.${pkg.name}.svg
  '';
})
