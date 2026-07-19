{ config, lib, pkgs, ... }:

let
  cfg = config.open-ot.opcua-fx-bridge;
in {
  options.open-ot.opcua-fx-bridge = {
    enable = lib.mkEnableOption "OPC UA Field eXchange ↔ Zenoh bridge (cross-vendor industrial interop)";

    bridgeBinary = lib.mkOption {
      type = lib.types.path;
      example = lib.literalExpression "/var/lib/open-ot/bin/open-ot-opcua-fx-bridge";
      description = ''
        Path to the bridge daemon binary. Built from a separate Rust crate
        (out of scope for this NixOS module — see roadmap entry "OPC UA FX
        bridge daemon" in `60-apps/etzhayyim-project-open-ot/PROTOTYPE-MICROGRID.md`).

        The daemon links against open62541 (Mozilla Public License v2,
        compatible with Apache-2.0 distribution). A reference build will
        live in `cells/../bridges/opcua-fx/` once the daemon is implemented.
      '';
    };

    listenInterface = lib.mkOption {
      type = lib.types.str;
      default = "lan0";
      description = ''
        Network interface bound to the OPC UA FX TSN ports. Typically the
        same TSN switch port used by Zenoh for the field-side fabric so the
        bridge can map subscriptions one-to-one without crossing the WAN.
      '';
    };

    opcUaPort = lib.mkOption {
      type = lib.types.port;
      default = 4840;
      description = "OPC UA TCP port (per IEC 62541 Part 6).";
    };

    zenohEndpoint = lib.mkOption {
      type = lib.types.str;
      default = "tcp/127.0.0.1:7447";
      description = "Zenoh router endpoint to bridge to (typically the local zenohd).";
    };

    keyMappings = lib.mkOption {
      type = lib.types.listOf (lib.types.submodule {
        options = {
          opcUaNodeId = lib.mkOption {
            type = lib.types.str;
            example = "ns=2;s=Pump1.SetPoint";
            description = "OPC UA NodeId (namespace + identifier).";
          };
          zenohKey = lib.mkOption {
            type = lib.types.str;
            example = "open-ot/site-a/te-01/setpoint/cv";
            description = "Zenoh key expression to bridge to / from.";
          };
          direction = lib.mkOption {
            type = lib.types.enum [ "opcuaToZenoh" "zenohToOpcua" "bidirectional" ];
            default = "bidirectional";
          };
          dataType = lib.mkOption {
            type = lib.types.enum [ "int32" "int64" "float32" "float64" "boolean" "string" ];
            description = ''
              Wire type. Note: OPC UA `Float`/`Double` cross the Zenoh
              boundary as float, but the open-ot cell side stores µ-units
              as integers — the bridge must apply the agreed scaling
              before publishing onto a Zenoh key bound to a cell signal.
            '';
          };
          scaleMicro = lib.mkOption {
            type = lib.types.nullOr lib.types.int;
            default = null;
            example = 1000000;
            description = ''
              Optional µ-unit scaling factor for analog signals. When set,
              float values from OPC UA are multiplied (and integer values
              going the other way are divided) by this constant. Aligns
              with the open-ot µ-unit wire convention (see SPEC §3).
            '';
          };
        };
      });
      default = [ ];
      description = "Static key mapping table. For dynamic mappings, the daemon supports a JSON config file; this option is the simple-case shortcut.";
    };

    securityMode = lib.mkOption {
      type = lib.types.enum [ "None" "Sign" "SignAndEncrypt" ];
      default = "SignAndEncrypt";
      description = ''
        OPC UA SecurityMode (per IEC 62541 Part 2). MUST stay at
        SignAndEncrypt for any cross-vendor link in production. `None`
        is permitted only on isolated bench networks and is rejected by
        the IEC 62443-3-3 SL-2 posture this project targets.
      '';
    };

    pkiDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/open-ot/opcua-pki";
      description = "Directory holding OPC UA application instance certificates + trust list (managed via `agenix` / `sops-nix`).";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.securityMode != "None"
          || (config.networking.firewall.enable == false);
        message = ''
          open-ot.opcua-fx-bridge.securityMode = "None" is only safe on a
          firewalled-off bench. Either disable the firewall (`networking.
          firewall.enable = false`) on a true air-gapped subnet, or raise
          securityMode to "Sign" / "SignAndEncrypt".
        '';
      }
    ];

    users.users.open-ot-opcua = {
      isSystemUser = true;
      group = "open-ot";
      description = "open-ot OPC UA FX bridge daemon";
    };

    systemd.tmpfiles.rules = [
      "d ${cfg.pkiDir} 0700 open-ot-opcua open-ot - -"
      "d /var/lib/open-ot/opcua-fx 0750 open-ot-opcua open-ot - -"
    ];

    environment.etc."open-ot/opcua-fx-mappings.json".text = builtins.toJSON {
      version = 1;
      listen_interface = cfg.listenInterface;
      opc_ua_port = cfg.opcUaPort;
      zenoh_endpoint = cfg.zenohEndpoint;
      security_mode = cfg.securityMode;
      pki_dir = cfg.pkiDir;
      mappings = cfg.keyMappings;
    };

    systemd.services.open-ot-opcua-fx-bridge = {
      description = "open-ot OPC UA FX ↔ Zenoh bridge";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" "zenohd.service" ];
      requires = [ "zenohd.service" ];

      serviceConfig = {
        Type = "simple";
        User = "open-ot-opcua";
        Group = "open-ot";
        ExecStart = ''
          ${cfg.bridgeBinary} \
            --config /etc/open-ot/opcua-fx-mappings.json \
            --pki-dir ${cfg.pkiDir}
        '';
        Restart = "always";
        RestartSec = "5s";

        # Hardening
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        ReadWritePaths = [ cfg.pkiDir "/var/lib/open-ot/opcua-fx" ];
        # OPC UA TCP needs to bind a privileged port only when port < 1024;
        # the IEC 62541 default is 4840 (unprivileged), so no caps needed.
      };
    };

    networking.firewall.allowedTCPPorts = [ cfg.opcUaPort ];
  };
}
