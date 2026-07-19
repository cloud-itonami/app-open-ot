{ config, lib, pkgs, ... }:

let
  cfg = config.open-ot.zenohd;
in {
  options.open-ot.zenohd = {
    enable = lib.mkEnableOption "Eclipse Zenoh router (data-plane substrate)";

    listen = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "tcp/0.0.0.0:7447" ];
      example = [ "tcp/0.0.0.0:7447" "udp/0.0.0.0:7447" "unixsock-stream:///run/zenoh.sock" ];
      description = "Zenoh listen endpoints. UDP on TSN ports for field-side; loopback unix-socket for orchestrator IPC.";
    };

    package = lib.mkOption {
      type = lib.types.package;
      # `zenoh` is in nixpkgs as `zenohd`; if missing on a particular
      # release, override per-deployment.
      default = pkgs.zenohd or pkgs.zenoh;
      description = "Zenoh router package.";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.zenohd = {
      isSystemUser = true;
      group = "zenohd";
      home = "/var/lib/zenohd";
      createHome = true;
    };
    users.groups.zenohd = { };

    systemd.services.zenohd = {
      description = "Eclipse Zenoh router (open-ot data plane)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        Type = "simple";
        User = "zenohd";
        Group = "zenohd";
        ExecStart = "${cfg.package}/bin/zenohd " +
          (lib.concatMapStringsSep " " (l: "--listen ${l}") cfg.listen);
        Restart = "always";
        RestartSec = "2s";

        # Hardening
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        ReadWritePaths = [ "/var/lib/zenohd" "/run" ];
        # Zenoh needs raw sockets for UDP discovery.
        AmbientCapabilities = [ "CAP_NET_BIND_SERVICE" ];
      };
    };

    networking.firewall.allowedTCPPorts = [ 7447 ];
    networking.firewall.allowedUDPPorts = [ 7447 ];
  };
}
