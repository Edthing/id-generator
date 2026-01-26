# ID Generator Helm Chart

A Helm chart for deploying the ID Generator service on Kubernetes with **automatic, native unique worker IDs**.

## Overview

This chart deploys the ID Generator as a **StatefulSet**. 

Unlike traditional setups that require sidecars or wrapper scripts, this application **natively detects its worker ID** using one of two methods:
1. **K8s Downward API (Preferred):** Reads the `apps.kubernetes.io/pod-index` label (available in K8s 1.28+).
2. **Hostname Parsing (Fallback):** Parses the pod hostname (e.g., `id-generator-1` -> Worker ID `1`).

This ensures safe, collision-free Snowflake ID generation without complex bootstrap logic.

## Prerequisites

- Kubernetes 1.23+
- Helm 3.0+
- (Optional) KEDA for event-driven autoscaling

## Installation

```bash
# Install from local directory
helm install my-id-generator ./chart
```

## Configuration

### Key Values

| Parameter | Description | Default |
|-----------|-------------|---------|
| `replicaCount` | Initial number of replicas | `3` |
| `image.repository` | Image repository | `ghcr.io/Edthing/id-generator` |
| `autoscaling.enabled` | Enable standard HPA | `true` |
| `autoscaling.keda.enabled` | Enable KEDA autoscaling (replaces HPA) | `true` |
| `autoscaling.minReplicas` | Minimum replicas | `2` |
| `autoscaling.maxReplicas` | Maximum replicas (keep within worker ID space) | `10` |
| `serviceMonitor.enabled` | Create Prometheus ServiceMonitor | `false` |

### Autoscaling: HPA vs. KEDA

**Standard HPA** scales based on CPU/Memory usage.

**KEDA** (if enabled) scales based on the custom metric `id_generator_sequence_exhausted_total`. This allows the cluster to proactively scale up when the generator is under heavy load and running out of sequence numbers within a millisecond, preventing ID generation failures.

### Example Custom Values (KEDA Enabled)

```yaml
# keda-values.yaml
replicaCount: 3

autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 20
  keda:
    enabled: true
    prometheusServerAddress: http://prometheus-k8s.monitoring.svc.cluster.local
    threshold: "5" # Scale if >5 failures per minute

resources:
  requests:
    memory: "128Mi"
    cpu: "250m"
  limits:
    memory: "256Mi"
    cpu: "1000m"
```

## How It Works

1. **StatefulSet** provides stable hostnames (`web-0`, `web-1`).
2. **Pod Startup:** The Rust binary starts and checks `WORKER_ID` env var.
   - The chart injects this via the Downward API.
   - If missing, the binary parses the hostname.
3. **Snowflake Algorithm:** The worker ID is used as the node segment of the 64-bit ID.
4. **Metrics:** The app exposes `/metrics` for Prometheus scraping.

## Uninstalling

```bash
helm uninstall my-id-generator
```
