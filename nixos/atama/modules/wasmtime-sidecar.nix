{ config, lib, pkgs, ... }:

let
  cfg = config.open-ot.wasmtime-sidecar;
in {
  options.open-ot.wasmtime-sidecar = {
    enable = lib.mkEnableOption "Wasmtime tier-2 cell host (co-located with langgraph orchestrator)";

    cellsBinDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/open-ot/cells";
      description = ''
        Directory holding signed `.wasm` artefacts. Populated at deploy
        time from the cells/ workspace `target/wasm32-unknown-unknown/release/`.
        `pinModule` records (per `com.etzhayyim.apps.openOt.pinModule`) reference
        artefacts here by content hash.
      '';
    };

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.wasmtime;
      description = "Wasmtime CLI package (for diagnostic loads; Python orchestrator uses `wasmtime-py` in-process).";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.open-ot = lib.mkIf (!config.users.users ? open-ot) {
      isSystemUser = true;
      group = "open-ot";
      home = "/var/lib/open-ot";
      createHome = true;
    };
    users.groups.open-ot = lib.mkIf (!config.users.groups ? open-ot) { };

    systemd.tmpfiles.rules = [
      "d ${cfg.cellsBinDir} 0750 open-ot open-ot - -"
    ];

    # The actual tier-2 cells run inside the langgraph-service process via
    # wasmtime-py. This module's purpose is the directory contract +
    # `wasmtime` CLI for ops debugging.
    environment.systemPackages = [ cfg.package ];
  };
}
