{
  description = "Giemon Atama edge controller — NixOS configuration for open-ot orchestrator";

  inputs = {
    # Pin to a recent NixOS release with PREEMPT_RT kernel package.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
  };

  outputs = { self, nixpkgs, ... }:
    let
      system = "aarch64-linux";  # RK3588
      pkgs = import nixpkgs { inherit system; };
    in {
      nixosConfigurations.atama = nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [
          ./configuration.nix
          ./modules/realtime-tuning.nix
          ./modules/zenohd.nix
          ./modules/checkpointer-client.nix
          ./modules/wasmtime-sidecar.nix
          ./modules/langgraph-service.nix
          ./modules/opcua-fx-bridge.nix
        ];
        specialArgs = {
          # Path to the open-ot orchestrator Python project. Override per
          # deployment if the source tree lives elsewhere.
          orchestratorSrc = ../../orchestrator;
          # Path to the cells workspace (for wasm artefacts).
          cellsSrc = ../../cells;
        };
      };
    };
}
