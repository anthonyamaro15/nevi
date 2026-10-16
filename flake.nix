{
  description = "Development environment and package for nevi, a Neovim-like terminal editor written in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));

      # Libraries nevi links against:
      #   - openssl / libssh2 / zlib: git2 -> libgit2-sys, libssh2-sys
      #   - libX11 / libxcb / wayland: arboard (system clipboard)
      nativeLibs =
        pkgs:
        let
          inherit (pkgs) lib stdenv;
        in
        [ pkgs.openssl pkgs.libssh2 pkgs.zlib ]
        ++ lib.optionals stdenv.hostPlatform.isLinux [
          pkgs.libx11
          pkgs.libxcb
          pkgs.libxcb-util
          pkgs.wayland
          pkgs.wayland-protocols
          pkgs.libxkbcommon
        ];
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          inherit (pkgs) lib rustPlatform;
          cargoToml = lib.importTOML ./Cargo.toml;
        in
        {
          default = rustPlatform.buildRustPackage {
            pname = cargoToml.package.name;
            version = cargoToml.package.version;

            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ pkgs.pkg-config pkgs.cmake ];
            buildInputs = nativeLibs pkgs;

            # Link against the nix-provided libssh2/openssl instead of
            # building the vendored copies.
            env.LIBSSH2_SYS_USE_PKG_CONFIG = "1";

            meta = {
              description = cargoToml.package.description;
              homepage = cargoToml.package.repository;
              license = lib.licenses.mit;
              mainProgram = "nevi";
              platforms = lib.platforms.unix;
            };
          };
        }
      );

      # Lets other flakes do:
      #   nixpkgs.overlays = [ nevi.overlays.default ];
      #   environment.systemPackages = [ pkgs.nevi ];
      overlays.default = final: _prev: {
        nevi = self.packages.${final.system}.default;
      };

      devShells = forAllSystems (
        pkgs:
        let
          inherit (pkgs) lib stdenv;
        in
        {
          default = pkgs.mkShell {
            name = "nevi";

            packages = with pkgs; [
              # Rust toolchain (rustc 1.98 > MSRV 1.85, edition 2024)
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer

              # Build tooling for native crates (cc comes from stdenv)
              pkg-config
              cmake
              gnumake

              # Developer conveniences
              git
              neovim # used by the `vim_oracle_smoke` parity test
            ]
            ++ lib.optionals stdenv.hostPlatform.isLinux (nativeLibs pkgs ++ [ pkgs.mold ])
            ++ lib.optionals stdenv.hostPlatform.isDarwin (nativeLibs pkgs);

            env = {
              RUST_BACKTRACE = "1";
              # Use the nix-provided libssh2/openssl instead of vendoring.
              LIBSSH2_SYS_USE_PKG_CONFIG = "1";
            }
            // lib.optionalAttrs stdenv.hostPlatform.isLinux {
              # Link with mold instead of GNU ld. mold is several times faster;
              # the binary is a drop-in replacement, so gcc stays the driver.
              RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
            };
          };
        }
      );
    };
}
