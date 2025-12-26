{ rustPlatform
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

rustPlatform.buildRustPackage rec {
  name = "deadman";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  runtimeInputs = [ systemd ];
  nativeBuildInputs = [ rustfmt pkg-config gcc glib ];
  buildInputs = [ gtk4 libadwaita cairo ];
  desktopEntry = makeDesktopItem {
    inherit name;
    desktopName = name;
    comment = "Systemd based usb device deadman kill switch";
    exec = "${placeholder "out"}/bin/deadman-gui";
    icon = "deadman";
    categories = [ "Utility" ];
  };
  postInstall = ''
    mkdir -p $out/share/applications
    cp -r $desktopEntry $out/share/applications/ 
  '';
}
