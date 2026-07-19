{ config, lib, pkgs, orchestratorSrc, ... }:

let
  cfg = config.open-ot.langgraph;

  # Build a Python environment from the orchestrator's pyproject.toml.
  # In a real deploy, `uv2nix` or `poetry2nix` would convert the lockfile
  # to a Nix derivation. For the spec, we declare the dependency surface;
  # a downstream overlay supplies the actual derivation.
  pythonPkg =
    if cfg.pythonVersion == "3.11" then pkgs.python311
    else if cfg.pythonVersion == "3.12" then pkgs.python312
    else throw "open-ot.langgraph.pythonVersion must be 3.11 or 3.12";

  pythonEnv = pythonPkg.withPackages (ps: with ps; [
    # Names align with `orchestrator/pyproject.toml`. Some packages may
    # require an overlay if not in nixpkgs at the pinned release.
    wasmtime
    langgraph
    asyncpg
    sqlalchemy
    granian
  ]);
in {
  options.open-ot.langgraph = {
    enable = lib.mkEnableOption "Pregel orchestrator (LangGraph + Granian) systemd service";

    pythonVersion = lib.mkOption {
      type = lib.types.enum [ "3.11" "3.12" ];
      default = "3.11";
      description = "Python interpreter version. 3.11 matches `40-engine/kotoba/crates/kotoba-kotodama/py` baseline.";
    };

    granianWorkers = lib.mkOption {
      type = lib.types.int;
      default = 2;
      description = "Number of Granian worker processes.";
    };

    bindAddress = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
    };

    bindPort = lib.mkOption {
      type = lib.types.port;
      default = 8080;
    };

    isolatedCpuPin = lib.mkOption {
      type = lib.types.str;
      default = "4-7";
      description = ''
        CPUs the service is pinned to via `AllowedCPUs`. Must be a subset
        of `open-ot.realtime.isolatedCpus`.
      '';
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

    systemd.services.open-ot-langgraph = {
      description = "open-ot Pregel orchestrator (LangGraph)";
      wantedBy = [ "multi-user.target" ];
      after = [
        "network.target"
        "zenohd.service"
      ];
      requires = [ "zenohd.service" ];

      environment = {
        # The checkpointer module writes /etc/open-ot/checkpointer.toml.
        OPEN_OT_CHECKPOINTER_CONFIG = "/etc/open-ot/checkpointer.toml";
        OPEN_OT_CELLS_DIR = "/var/lib/open-ot/cells";
        # Disable Python bytecode caching to keep startup latency
        # predictable on cold boot.
        PYTHONDONTWRITEBYTECODE = "1";
      };

      serviceConfig = {
        Type = "simple";
        User = "open-ot";
        Group = "open-ot";
        WorkingDirectory = orchestratorSrc;

        # Granian + uvicorn-style invocation. The orchestrator exposes a
        # control-plane HTTP surface (per ADR-2605080600 §LangServer
        # Granian L3 Runtime) at bindAddress:bindPort.
        ExecStart = ''
          ${pythonEnv}/bin/granian \
            --interface asgi \
            --workers ${toString cfg.granianWorkers} \
            --host ${cfg.bindAddress} \
            --port ${toString cfg.bindPort} \
            open_ot_orchestrator.app:app
        '';

        Restart = "always";
        RestartSec = "5s";

        # Real-time: lock memory + run on isolated CPUs.
        LimitMEMLOCK = "infinity";
        LimitRTPRIO = 99;
        AllowedCPUs = cfg.isolatedCpuPin;

        # Hardening
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        ReadWritePaths = [ "/var/lib/open-ot" "/run" ];
      };
    };

    # Ops surface — open the bind port on loopback only by default.
    networking.firewall.interfaces.lo.allowedTCPPorts = [ cfg.bindPort ];
  };
}
