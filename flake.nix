{
  description = "idx: Indonesian stock market (IDX / IHSG) CLI and MCP server for people and AI agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      rust-overlay,
    }:
    let
      inherit (nixpkgs) lib;
      # No x86_64-darwin: nixpkgs 26.05 is the last release that builds it.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      inherit (cargoToml.package) version;
      # Cargo.toml's rust-version ("1.89") as a toolchain release ("1.89.0").
      msrvRelease = lib.versions.pad 3 cargoToml.package.rust-version;

      perSystem = lib.genAttrs systems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          # rust-toolchain.toml (stable, rustfmt, clippy) is the one toolchain source.
          toolchainFor = p: p.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (crane.mkLib pkgs).overrideToolchain toolchainFor;
          # The declared minimum (Cargo.toml rust-version), for checks.msrv.
          msrvCraneLib = (crane.mkLib pkgs).overrideToolchain (
            p: p.rust-bin.stable.${msrvRelease}.minimal
          );

          # What the crate compiles and its tests read (Cargo.toml `include`
          # without the docs): sources, the bundled CSVs, the recorded fixtures.
          # Add a path here if the crate starts embedding files from elsewhere.
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./rustfmt.toml
              ./src
              ./data
              ./tests
            ];
          };

          commonArgs = {
            inherit src version;
            pname = cargoToml.package.name;
            strictDeps = true;
            # rusqlite's bundled SQLite and ring build their C with the stdenv
            # compiler; rustls runs on ring with webpki-roots, so no OpenSSL,
            # pkg-config or system CA bundle.
            buildInputs = lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
          };

          # Dependencies are built once per profile: release for the package,
          # dev for clippy and the tests (the profile CI uses). The release
          # set only feeds `cargo build`, so it skips buildDepsOnly's default
          # `cargo check` pass, which only clippy would reuse.
          cargoArtifacts = craneLib.buildDepsOnly (
            commonArgs
            // {
              doCheck = false;
              buildPhaseCargoCommand = "cargoWithProfile build --locked";
            }
          );
          devArgs = commonArgs // {
            CARGO_PROFILE = "dev";
          };
          devArtifacts = craneLib.buildDepsOnly devArgs;

          idx = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              doCheck = false; # checks.test runs the suite
              nativeBuildInputs = [ pkgs.installShellFiles ];
              postInstall = lib.optionalString (pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform) ''
                export HOME="$TMPDIR"
                installShellCompletion --cmd idx \
                  --bash <($out/bin/idx completions bash) \
                  --fish <($out/bin/idx completions fish) \
                  --zsh <($out/bin/idx completions zsh)
              '';
              meta = {
                description = "Indonesian stock market (IDX / IHSG) CLI and MCP server";
                homepage = "https://ihsg.midoriresearch.com";
                license = lib.licenses.mit;
                mainProgram = "idx";
                platforms = systems;
              };
            }
          );

          # `nix run github:fzn0x/idx#mcp`: the MCP server over stdio, for agent configs.
          idx-mcp = pkgs.writeShellApplication {
            name = "idx-mcp";
            text = ''exec ${lib.getExe idx} mcp "$@"'';
          };

          checks = {
            inherit idx;
            fmt = craneLib.cargoFmt { inherit src; };
            clippy = craneLib.cargoClippy (
              devArgs
              // {
                cargoArtifacts = devArtifacts;
                cargoClippyExtraArgs = "--all-targets -- -D warnings";
                doInstallCargoArtifacts = false; # nothing builds on top; keep the store small
              }
            );
            test = craneLib.cargoTest (
              devArgs
              // {
                cargoArtifacts = devArtifacts;
                cargoTestExtraArgs = "--all-targets";
                doInstallCargoArtifacts = false;
                # The tests point config and cache at temp dirs; nothing may
                # fall back to the sandbox's unwritable /homeless-shelter.
                preBuild = ''export HOME="$TMPDIR"'';
              }
            );
            # The declared minimum Rust (Cargo.toml rust-version) still builds
            # the locked dependency tree, tests included.
            msrv = msrvCraneLib.mkCargoDerivation (
              commonArgs
              // {
                pnameSuffix = "-msrv";
                cargoArtifacts = null;
                doInstallCargoArtifacts = false;
                buildPhaseCargoCommand = "cargo check --locked --all-targets";
              }
            );
            # The packaged binary and the MCP app start, report their version,
            # keep the offline exit-code contract (release.yml smokes the same
            # way) and answer an MCP handshake over stdio.
            smoke = pkgs.runCommand "idx-smoke" { } ''
              export HOME="$TMPDIR"
              ${lib.getExe idx} version
              ${lib.getExe idx} -o json version > version.json
              grep -q '"version": "${version}"' version.json

              status=0
              ${lib.getExe idx} --offline -o json stocks quote BBCA 2>err.json || status=$?
              test "$status" -eq 5
              grep -q '"code":"offline_miss"' err.json

              {
                echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"nix-smoke","version":"0"}}}'
                echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'
                echo '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
                sleep 2
              } | IDX_OFFLINE=1 timeout 60 ${lib.getExe idx-mcp} > mcp.jsonl
              # Name and version only, so a field rmcp or get_info adds later
              # (title, description, icons, websiteUrl) does not fail it.
              grep -q '"serverInfo":{"name":"idx",[^}]*"version":"${version}"' mcp.jsonl
              grep -q '"name":"idx_quote"' mcp.jsonl

              touch "$out"
            '';
          };

          # The same toolchain plus what an editor needs.
          devCraneLib = (crane.mkLib pkgs).overrideToolchain (
            p:
            (toolchainFor p).override {
              extensions = [
                "rustfmt"
                "clippy"
                "rust-src"
                "rust-analyzer"
              ];
            }
          );
        in
        {
          inherit idx idx-mcp checks;
          devShell = devCraneLib.devShell {
            inherit (commonArgs) buildInputs;
            # web/ (node --test web/test/*.mjs, npx wrangler) and npm/.
            packages = [ pkgs.nodejs_22 ];
          };
        }
      );
    in
    {
      packages = lib.mapAttrs (_: s: {
        default = s.idx;
        inherit (s) idx idx-mcp;
      }) perSystem;

      apps = lib.mapAttrs (_: s: {
        default = {
          type = "app";
          program = lib.getExe s.idx;
          meta.description = "idx CLI";
        };
        mcp = {
          type = "app";
          program = lib.getExe s.idx-mcp;
          meta.description = "idx MCP server over stdio";
        };
      }) perSystem;

      checks = lib.mapAttrs (_: s: s.checks) perSystem;

      devShells = lib.mapAttrs (_: s: { default = s.devShell; }) perSystem;

      overlays.default = final: _prev: {
        idx = self.packages.${final.stdenv.hostPlatform.system}.idx;
      };
    };
}
