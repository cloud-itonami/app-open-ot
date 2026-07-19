{ config, pkgs, lib, ... }:

{
  # -------------------------------------------------------------------------
  # Identity + boot
  # -------------------------------------------------------------------------
  networking.hostName = "atama";
  networking.firewall.enable = true;

  time.timeZone = "Asia/Tokyo";

  # -------------------------------------------------------------------------
  # Module activations — option values defined here, services in modules/.
  # -------------------------------------------------------------------------

  open-ot.realtime = {
    enable = true;
    # Pin Cortex-A76 cores 4–7 for control-plane work; A55 cores 0–3 stay
    # for OS housekeeping.
    isolatedCpus = [ 4 5 6 7 ];
    # cyclictest target on isolated CPUs: ≤ 30 µs worst-case (per
    # cad-spec/giemon-atama/SPEC.md §5).
  };

  open-ot.zenohd = {
    enable = true;
    listen = [ "tcp/0.0.0.0:7447" "udp/0.0.0.0:7447" ];
    # Field-side TSN ports + WAN port participate; loopback for in-process
    # cells via `wasmtime-sidecar`.
  };

  open-ot.checkpointer = {
    enable = true;
    # Hyperdrive endpoint per ADR-0048 / 50-infra/vultr/risingwave.
    # Override via secret indirection (`agenix` / `sops-nix`) — never
    # commit a real DSN here.
    dsn = "@@OPEN_OT_RW_DSN@@";
    schema = "graphar";
    loopCheckpointTable = "vertex_open_ot_loop_checkpoint";
    signalChangeTable = "vertex_open_ot_signal_change";
  };

  open-ot.wasmtime-sidecar = {
    enable = true;
    cellsBinDir = "/var/lib/open-ot/cells";  # populated by deploy step
  };

  open-ot.langgraph = {
    enable = true;
    # Path to the orchestrator project; resolved at build time via flake
    # specialArgs.
    pythonVersion = "3.11";
    granianWorkers = 2;
    bindAddress = "127.0.0.1";
    bindPort = 8080;
  };

  # -------------------------------------------------------------------------
  # SSH / admin
  # -------------------------------------------------------------------------
  services.openssh = {
    enable = true;
    settings.PasswordAuthentication = false;
  };

  users.users.opsadmin = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    openssh.authorizedKeys.keys = [
      # populate per-deployment via overlay
    ];
  };

  system.stateVersion = "25.05";
}
