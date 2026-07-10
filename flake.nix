{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-parts,
      ...
    }@inputs:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      perSystem =
        {
          lib,
          pkgs,
          system,
          ...
        }:
        let
          package = pkgs.rustPlatform.buildRustPackage {
            pname = "wayvr";
            version = "next";

            src = lib.cleanSourceWith {
              src = ./.;
              filter =
                name: type:
                let
                  baseName = builtins.baseNameOf (builtins.toString name);
                in
                (lib.cleanSourceFilter name type)
                && !(
                  (lib.hasPrefix "flake" baseName)
                  || (type == "directory" && baseName == "target")
                  || baseName == ".envrc"
                );
            };

            cargoLock = {
              lockFile = ./Cargo.lock;
              allowBuiltinFetchGit = true;
            };

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.rustPlatform.bindgenHook
              pkgs.autoPatchelfHook
              # for whisper
              pkgs.cmake
              pkgs.shaderc
            ];
            buildInputs = pkgs.wayvr.buildInputs ++ [
              # extra buildInputs here
              pkgs.udev
              pkgs.libinput

              # for whisper
              pkgs.vulkan-headers
            ];

            inherit (pkgs.wayvr) postInstall preFixup;

            doCheck = false;

            env = {
              inherit (pkgs.wayvr) SHADERC_LIB_DIR;
            };
          };

        in
        {
          formatter = pkgs.nixfmt;

          packages.default = package;

          devShells.default = pkgs.mkShell {
            inputsFrom = [ package ];
            packages = [
              # Extra packages for development
              pkgs.rustfmt
              pkgs.clippy
              pkgs.rust-analyzer

              # used at runtime
              pkgs.procps
              pkgs.pulseaudio
              pkgs.xwayland-satellite
            ];
            env = {
              inherit (package) SHADERC_LIB_DIR;
              RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

              # unfortunately duplicated from the Nixpkgs recipe, but IDK a more graceful
              # way to do this.
              LD_LIBRARY_PATH = lib.makeLibraryPath [
                # X dependencies are dlopen'd at runtime by uidev
                pkgs.libx11
                pkgs.libxext
                pkgs.libxrandr
                pkgs.libxcb
                pkgs.libxkbcommon
                pkgs.openssl
                pkgs.openxr-loader
                pkgs.pipewire

                # only dlopen'd at runtime by uidev
                pkgs.libxcursor
                pkgs.libxi
                pkgs.wayland
                pkgs.vulkan-loader
              ];
            };
          };
        };
    };
}
