{ config, lib, pkgs, ... }:

let
  cfg = config.open-ot.realtime;
in {
  options.open-ot.realtime = {
    enable = lib.mkEnableOption "PREEMPT_RT kernel + RT tuning for control-plane workload";

    isolatedCpus = lib.mkOption {
      type = lib.types.listOf lib.types.int;
      default = [ ];
      example = [ 4 5 6 7 ];
      description = ''
        CPU cores to isolate from the kernel scheduler. Control-plane
        threads (LangGraph orchestrator + Wasmtime sidecar) pin here via
        SCHED_FIFO. Cores 0–3 (A55) stay for OS housekeeping; cores 4–7
        (A76) are typically isolated.
      '';
    };

    timerCpu = lib.mkOption {
      type = lib.types.int;
      default = 0;
      description = "CPU to keep periodic kernel work on (rcu / softirq).";
    };
  };

  config = lib.mkIf cfg.enable {
    # PREEMPT_RT mainline kernel package. Hardware tier ARM64; nixpkgs
    # exposes `linuxPackages_rt` from 25.05 onwards for aarch64.
    boot.kernelPackages = pkgs.linuxPackages_rt;

    boot.kernelParams = [
      # Take the listed CPUs out of the general scheduler.
      "isolcpus=${lib.concatMapStringsSep "," toString cfg.isolatedCpus}"
      # Pin the periodic tick to a single CPU; full-nohz on the rest.
      "nohz_full=${lib.concatMapStringsSep "," toString cfg.isolatedCpus}"
      "rcu_nocbs=${lib.concatMapStringsSep "," toString cfg.isolatedCpus}"
      # Keep RCU callbacks off isolated cores.
      "rcu_nocb_poll"
      # Less jittery tickless idle.
      "skew_tick=1"
      # Disable mitigations on a fixed-purpose embedded system if the
      # threat model permits; left commented because that's a per-deploy
      # decision.
      # "mitigations=off"
    ];

    # IRQ affinity — keep all hardware interrupts off the isolated cores.
    services.irqbalance.enable = false;
    systemd.services."irq-affinity-pin" = {
      description = "Pin all IRQs off isolated cores";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = let
        nonIsolated = lib.subtractLists cfg.isolatedCpus [ 0 1 2 3 4 5 6 7 ];
        mask = lib.foldl' (acc: cpu: acc + (lib.toHexString (lib.toInt (builtins.div (lib.pow 2 cpu) 1)))) "" nonIsolated;
      in ''
        # Set default IRQ affinity to non-isolated cores (cores 0-3 by default).
        # Bitmask: bits set for cores allowed to receive interrupts.
        for irq in /proc/irq/[0-9]*; do
          echo 0f > "$irq/smp_affinity" 2>/dev/null || true
        done
      '';
    };

    # Userspace governor + lock CPU frequencies high for predictable timing.
    powerManagement.cpuFreqGovernor = "performance";

    # Sysctl tunings for low-jitter operation.
    boot.kernel.sysctl = {
      "kernel.sched_rt_runtime_us" = -1;   # let SCHED_FIFO use 100 % when needed
      "kernel.sched_rt_period_us" = 1000000;
      "vm.swappiness" = 1;                 # avoid paging
      "vm.dirty_ratio" = 5;
      "vm.dirty_background_ratio" = 2;
    };

    # Lock memory limits for the orchestrator user.
    security.pam.loginLimits = [
      { domain = "open-ot"; type = "soft"; item = "rtprio"; value = "99"; }
      { domain = "open-ot"; type = "hard"; item = "rtprio"; value = "99"; }
      { domain = "open-ot"; type = "soft"; item = "memlock"; value = "unlimited"; }
      { domain = "open-ot"; type = "hard"; item = "memlock"; value = "unlimited"; }
    ];
  };
}
