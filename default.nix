{ rustPlatform, systemd }:

rustPlatform.buildRustPackage rec {
  name = "deadman";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  runtimeInputs = [ systemd ];
}
