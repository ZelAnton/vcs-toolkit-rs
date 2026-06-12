{
  description = "vcs-toolkit-rs: a typed Rust wrapper over git/jj + GitHub/GitLab/Gitea. The flagship output is `vcs-mcp` (the MCP server binary); per-crate packages expose every workspace crate.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;
        rustPlatform = pkgs.rustPlatform;

        # The toolchain comes from nixpkgs — whatever stable rustc/cargo
        # nixpkgs ships at evaluation time. Currently 1.95.0; moves with
        # `nix flake update`. MSRV verification (cargo check on 1.88.0) is
        # the release workflow's job, not the dev shell's — see
        # .github/workflows/ci.yml's `msrv` job.
        stableRust = pkgs.rustc;
        stableClippy = pkgs.clippy;
        stableRustfmt = pkgs.rustfmt;
        stableCargo = pkgs.cargo;

        # Filter the local source: drop .git/ and target/ (Nix cannot rebuild
        # inside a filtered source path, so the .git directory is dead weight
        # and target/ is rebuilt anyway). Everything else — Cargo.lock,
        # crates/*, cliff.toml, deny.toml, docs/ — is needed by the build.
        src = builtins.path {
          name = "vcs-toolkit-rs-src";
          path = ./.;
          filter = path: _type:
            let name = baseNameOf (toString path);
            in !(builtins.elem name [ ".git" "target" ]);
        };

        # Pull a crate's version from its Cargo.toml at evaluation time. Stays
        # in lockstep with the manifest (and the release workflow's
        # `cargo set-version` bumps) — no drift between Cargo.toml and the
        # flake's `version` field.
        readVersion = crate:
          let manifest = builtins.fromTOML
                           (builtins.readFile (./. + "/crates/${crate}/Cargo.toml"));
          in if manifest ? package.version
             then manifest.package.version
             else throw "crates/${crate}/Cargo.toml has no [package].version";

        # Runtime CLIs the MCP server (and watch) shell out to. The toolkit's
        # `processkit`-backed clients wrap these.
        runtimeCli = with pkgs;
          let jj = pkgs.jj or pkgs.jujutsu or null;
          in lib.filter (p: p != null) [
            pkgs.git
            jj
            pkgs.gh
            pkgs.glab
            (pkgs.tea or null)
          ];

        # Common attrs every crate build needs.
        commonCrate = { pname, version, ... }@args:
          rustPlatform.buildRustPackage (args // {
            pname = "vcs-${pname}";
            inherit version src;
            cargoLock = { lockFile = ./Cargo.lock; };
            buildInputs = [ ];
            doCheck = false;
          });

        # Library-only crate: cargo produces .rlib artifacts. We hand-roll the
        # install phase so the result is useful to a downstream Rust consumer
        # in a Nix-only world — they can link against the .rlib in $out/deps.
        mkLibCrate = crate: commonCrate {
          pname = crate;
          version = readVersion crate;
          installPhase = ''
            runHook preInstall
            mkdir -p $out
            cp -R target/release/build $out/build
            cp -R target/release/deps $out/deps
            cp -R target/release/.fingerprint $out/.fingerprint
            runHook postInstall
          '';
          meta = with lib; {
            license = licenses.mit;
            description = "Library crate vcs-${crate} (built from workspace source)";
            platforms = platforms.unix;
          };
        };

        # Binary-bearing crate: copy the produced executable(s) into $out/bin
        # and optionally wrap them with runtime dependencies on PATH.
        mkBinCrate = crate: { bins, wrap ? null }:
          let base = commonCrate {
            pname = crate;
            version = readVersion crate;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            installPhase = ''
              runHook preInstall
              mkdir -p $out/bin
              ${lib.concatMapStringsSep "\n"
                (b: ''cp target/release/${b} $out/bin/${b}'') bins}
              runHook postInstall
            '';
          };
          in if wrap == null
             then base
             else base.overrideAttrs (old: {
               postInstall = (old.postInstall or "") + ''
                 wrapProgram $out/bin/${bins.${"${crate}"}} \
                   --prefix PATH : ${lib.makeBinPath wrap}
               '';
             });

        vcs-mcp = (commonCrate {
          pname = "mcp";
          version = readVersion "mcp";
          nativeBuildInputs = [ pkgs.makeWrapper ];
          installPhase = ''
            runHook preInstall
            mkdir -p $out/bin
            # `cargoBuildHook` builds with `--target x86_64-unknown-linux-gnu`,
            # so the bin lives at target/<triple>/release/, not target/release/.
            # `find` is the standard nixpkgs idiom for this — it doesn't break
            # on the host (no triple) or cross builds (different triple).
            bin=$(find target -type f -name vcs-mcp -executable | head -1)
            if [ -z "$bin" ]; then
              echo "vcs-mcp: built binary not found under target/"
              find target -type f -name 'vcs-mcp*' || true
              exit 1
            fi
            cp "$bin" $out/bin/vcs-mcp
            runHook postInstall
          '';
        }).overrideAttrs (old: {
          postInstall = (old.postInstall or "") + ''
            wrapProgram $out/bin/vcs-mcp \
              --prefix PATH : ${lib.makeBinPath runtimeCli}
          '';
          meta = with lib; {
            description = "MCP server exposing vcs-core/vcs-forge operations as tools";
            license = licenses.mit;
            mainProgram = "vcs-mcp";
            platforms = platforms.unix;
          };
        });

        vcs-watch = (commonCrate {
          pname = "watch";
          version = readVersion "watch";
          nativeBuildInputs = [ pkgs.makeWrapper ];
          installPhase = ''
            runHook preInstall
            mkdir -p $out/bin
            bin=$(find target -type f -name vcs-watch -executable | head -1)
            if [ -z "$bin" ]; then
              echo "vcs-watch: built binary not found under target/"
              find target -type f -name 'vcs-watch*' || true
              exit 1
            fi
            cp "$bin" $out/bin/vcs-watch
            runHook postInstall
          '';
        }).overrideAttrs (old: {
          postInstall = (old.postInstall or "") + ''
            wrapProgram $out/bin/vcs-watch \
              --prefix PATH : ${lib.makeBinPath (with pkgs; [ git (pkgs.jj or pkgs.jujutsu) ])}
          '';
          meta = with lib; {
            description = "Filesystem watcher over vcs-core repos, streaming typed state-change events";
            license = licenses.mit;
            mainProgram = "vcs-watch";
            platforms = platforms.unix;
          };
        });

        packages = {
          vcs-diff        = mkLibCrate "diff";
          vcs-cli-support = mkLibCrate "cli-support";
          vcs-git         = mkLibCrate "git";
          vcs-jj          = mkLibCrate "jj";
          vcs-github      = mkLibCrate "github";
          vcs-gitlab      = mkLibCrate "gitlab";
          vcs-gitea       = mkLibCrate "gitea";
          vcs-forge       = mkLibCrate "forge";
          vcs-testkit     = mkLibCrate "testkit";
          vcs-core        = mkLibCrate "core";
          inherit vcs-watch vcs-mcp;
          default = vcs-mcp;
        };

        apps = {
          vcs-mcp = {
            type = "app";
            program = "${vcs-mcp}/bin/vcs-mcp";
          };
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ vcs-mcp ];
          packages = [
            stableRust
            stableClippy
            stableRustfmt
            stableCargo
            vcs-mcp
            pkgs.cargo-edit
            pkgs.git-cliff
            pkgs.cargo-deny
            pkgs.rust-analyzer
            pkgs.gnumake
            pkgs.pkg-config
          ];
          shellHook = ''
            # `RUSTUP_TOOLCHAIN` is set for the rare case someone has a
            # rustup-managed toolchain that nixpkgs doesn't ship; most
            # users can ignore it. The dev shell's nixpkgs-pinned toolchain
            # wins by being first on PATH.
            export RUSTUP_TOOLCHAIN=${pkgs.rustc.version}
          '';
        };

        checks.cargo-test = rustPlatform.buildRustPackage {
          pname = "vcs-toolkit-cargo-test";
          version = "check";
          inherit src;
          cargoLock = { lockFile = ./Cargo.lock; };
          buildInputs = runtimeCli;
          # Skip the build phase — the check exists to run tests, not to
          # produce a binary. `cargo test` does both compilation and test
          # execution, so an empty build is fine.
          buildPhase = "true";
          doCheck = true;
          checkPhase = ''
            runHook preCheck
            cargo test --workspace --all-features
            runHook postCheck
          '';
          # An empty `$out` satisfies nix's "build must produce a $out" rule
          # without leaving a useless artifact on disk.
          installPhase = "mkdir -p $out";
        };
      in
      { inherit packages apps devShells checks; }
    );
}
