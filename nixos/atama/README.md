# Giemon Atama — NixOS module spec

Declarative NixOS configuration for the open-ot **Atama (頭)** edge controller (per `cad-spec/giemon-atama/SPEC.md` — RK3588 / 16 GB LPDDR5 / 4-port TSN switch). Hosts the Pregel orchestrator stack: LangGraph + Wasmtime sidecar + Zenoh router + RisingWave checkpointer client.

**Status (2026-05-15)**: spec only. No live Atama HW; no `nixos-rebuild` validation has been run. Ready for review and Q3 2026 deployment per ADR-2605151200 §R4.

## Layout

```
nixos/atama/
├── README.md                          ← you are here
├── flake.nix                          top-level flake; pins nixpkgs + builds NixosConfigurations.atama
├── configuration.nix                  example host config that imports all modules
└── modules/
    ├── realtime-tuning.nix            PREEMPT_RT kernel selection + isolcpus / irqaffinity / sysctl
    ├── langgraph-service.nix          systemd service hosting the Python orchestrator
    ├── wasmtime-sidecar.nix           tier-2 cell host (co-located with langgraph)
    ├── zenohd.nix                     Eclipse Zenoh router (data-plane substrate)
    ├── checkpointer-client.nix        RisingWave asyncpg / Hyperdrive client config
    └── opcua-fx-bridge.nix            OPC UA Field eXchange ↔ Zenoh bridge (cross-vendor interop)
```

## Usage (post-Risk-1 deploy)

```bash
# On a development workstation with nix flakes enabled:
cd 60-apps/etzhayyim-project-open-ot/nixos/atama
nix flake check                                    # syntax + module evaluation
nix build .#nixosConfigurations.atama.config.system.build.toplevel

# On the Atama (after first-boot install via image):
sudo nixos-rebuild switch --flake .#atama
```

## Module dependency graph

```
configuration.nix
  ├── realtime-tuning           (kernel + isolcpus must come first)
  ├── zenohd                    (substrate must be up before langgraph)
  ├── checkpointer-client       (RW connection must be reachable before langgraph)
  ├── wasmtime-sidecar          (tier-2 cell host, depends on zenohd)
  └── langgraph-service         (orchestrator, depends on all above)
```

## What's intentionally NOT here

- **Hardware-specific bits** (TSN switch silicon driver, eMMC partition layout, UPS firmware) — those belong in a separate Atama Rev-1 hardware bring-up package.
- **OPC UA FX bridge** — separate module, post-MVP+1.
- **HMI / Svelte editor service** — gated on Risk-1 PASS per ADR §R2.
- **Secret material** (RW credentials, builder DID signing key) — managed via `agenix` or `sops-nix`, defined per-deployment, not in this spec.

## Decision log

- **Why not Talos**: Atama is a single-tenant edge controller, not a K8s node. Talos shines when you already run K8s; here NixOS is preferred for declarative IaC + snapshot/rollback + the etzhayyim team's existing Nix culture (per ADR §R1). Talos remains the alternative for sites where the edge runs as a K8s node.
- **Why systemd over a custom supervisor**: systemd's unit dependency graph + restart policy + journal integration are sufficient for our 4 services. The orchestrator process model is already supervisor-shaped (LangGraph agent loop), no nested supervision needed.
- **Why no container per service**: the orchestrator + sidecars share state via local sockets / shared memory; Wasmtime is in-process; Zenoh router uses shm transport. Containerising would defeat shm. NixOS isolation via `DynamicUser` + systemd `ProtectSystem=strict` covers the security need without the IPC cost.
