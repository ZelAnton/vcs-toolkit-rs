{
  description = "vcs-toolkit-rs: typed Rust wrappers over git/jj + GitHub/GitLab/Gitea. Flagship output: the `vcs-mcp` MCP server binary.";

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

        # Toolchain note: the flake uses nixpkgs's own `rustc` (whatever
        # stable version nixpkgs-unstable ships at evaluation time — 1.95.0
        # at the time of writing). The workspace's MSRV is 1.88 (declared
        # in `[workspace.package].rust-version`); nixpkgs always exceeds
        # the MSRV, so the build itself is fine.

        # Filter the local source: drop .git/ (Nix can't rebuild inside a
        # filtered source path, so the .git directory is dead weight) and
        # target/ (rebuilt by cargo anyway). Everything else — Cargo.lock,
        # crates/*, cliff.toml, deny.toml, docs/ — is needed by the build.
        src = builtins.path {
          name = "vcs-toolkit-rs-src";
          path = ./.;
          filter = path: _type:
            !(builtins.elem (baseNameOf (toString path)) [ ".git" "target" ]);
        };

        # Read a crate's version from its Cargo.toml at evaluation time. Stays
        # in lockstep with the manifest (and the release workflow's
        # `cargo set-version` bumps) — no drift between Cargo.toml and the
        # flake's `version` field.
        readVersion = crate:
          (builtins.fromTOML
            (builtins.readFile (./. + "/crates/${crate}/Cargo.toml"))).package.version;

        # Runtime CLIs the MCP server shells out to. The toolkit's
        # `processkit`-backed clients wrap these. `jujutsu`'s binary is `jj`.
        # `tea` (Gitea) may be absent in some nixpkgs — filtered out, in
        # which case the MCP server's Gitea tools won't find `tea` on PATH
        # and will fail at runtime; configure it out of band if needed.
        runtimeCli = lib.filter (p: p != null) [
          pkgs.git
          (pkgs.jujutsu or null)
          pkgs.gh
          pkgs.glab
          (pkgs.tea or null)
        ];

        # The MCP server. `cargoBuildFlags` scopes cargo to the vcs-mcp
        # package + its bin, so we don't rebuild the full workspace in this
        # derivation. The vcs-core/vcs-forge path deps are still resolved
        # from `crates/` via `src`. `buildRustPackage`'s default install
        # handles the `--target <triple>` path (no `find` hack needed).
        vcs-mcp = rustPlatform.buildRustPackage {
          pname = "vcs-mcp";
          version = readVersion "mcp";
          inherit src;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "-p" "vcs-mcp" "--bin" "vcs-mcp" ];
          doCheck = false; # tests run in CI (ci.yml) on real runners, not here
          nativeBuildInputs = [ pkgs.makeWrapper ];
          postInstall = ''
            wrapProgram $out/bin/vcs-mcp \
              --prefix PATH : ${lib.makeBinPath runtimeCli}
          '';
          meta = with lib; {
            description = "MCP server exposing vcs-core/vcs-forge operations as agent tools";
            license = licenses.mit;
            mainProgram = "vcs-mcp";
            platforms = platforms.unix;
          };
        };
      in
      {
        packages = { inherit vcs-mcp; default = vcs-mcp; };

        apps.vcs-mcp = {
          type = "app";
          program = lib.getExe vcs-mcp;
        };
        apps.default = self.apps.${system}.vcs-mcp;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ vcs-mcp ];
          packages = [
            pkgs.rustc pkgs.cargo pkgs.clippy pkgs.rustfmt pkgs.rust-analyzer
            pkgs.cargo-edit pkgs.git-cliff pkgs.cargo-deny
          ] ++ runtimeCli;
        };
        # No custom `checks` here: the workspace test suite needs real
        # git/jj/gh on PATH and runs in CI (ci.yml). `nix flake check`
        # still builds every package (incl. vcs-mcp) and the devShell.
      }
    );
}
