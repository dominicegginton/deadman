{ rustPlatform
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
}
