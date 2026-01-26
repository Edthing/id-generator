# id-generator

A Rust web service implementing Twitter's Snowflake ID generation algorithm. Generates unique, time-sorted 64-bit identifiers suitable for distributed systems.

## Installation

### Helm Chart

You can install the chart directly from the GitHub release:

```bash
helm install my-id-generator https://github.com/Edthing/id-generator/releases/download/id-generator-0.1.0/id-generator-0.1.0.tgz --set autoscaling.keda.enabled=true
```

## Features

- Single ID generation endpoint
- Bulk ID generation (up to 4,096,000 IDs per request)
- Health check endpoint for container orchestration
- Handles clock drift and leap seconds with timeout protection
- Thread-safe sequence management
- Configurable worker threads
- Comprehensive input validation

## API Endpoints

### GET /id

Returns a single unique ID.

```json
{"id": "123456789012345678"}
```

### GET /ids/{count}

Returns multiple unique IDs (count must be 1 to 4,096,000).

```json
{"ids": ["123456789012345678", "123456789012345679", ...]}
```

### GET /health

Health check endpoint for container orchestration (Kubernetes, Docker, etc.).

```json
{"status": "healthy", "worker_id": 1}
```

### GET /metrics

Prometheus metrics endpoint.

```text
# HELP id_generator_ids_generated_total Total IDs generated
# TYPE id_generator_ids_generated_total counter
id_generator_ids_generated_total 123
# HELP id_generator_sequence_exhausted_total Times sequence was exhausted within a millisecond
# TYPE id_generator_sequence_exhausted_total counter
id_generator_sequence_exhausted_total 0
# HELP id_generator_worker_id Worker ID of this instance
# TYPE id_generator_worker_id gauge
id_generator_worker_id 1
```

### Error Responses

All errors return JSON with a consistent format:

```json
{"error": "Error message description"}
```

## Configuration

| Environment Variable | Description | Required | Default |
|---------------------|-------------|----------|---------|
| `WORKER_ID` | Unique worker identifier (0-1023) | Yes | - |
| `WORKERS` | Number of HTTP worker threads | No | 1 |

## Kubernetes / Helm

The included Helm chart supports both standard HPA (CPU/Memory) and KEDA-based autoscaling (based on sequence exhaustion).

### KEDA Autoscaling

To enable KEDA autoscaling:

1. Ensure KEDA is installed in your cluster.
2. Set `autoscaling.keda.enabled=true` in `values.yaml`.
3. Configure `autoscaling.keda.prometheusServerAddress` to point to your Prometheus instance.

The scaler monitors the `rate(id_generator_sequence_exhausted_total[1m])` metric. If the sequence limit (4096 IDs/ms) is hit frequently, KEDA will scale up the number of pods.

## Running

### Docker

```bash
docker run -d -p 8080:8080 -e WORKER_ID=1 ghcr.io/Edthing/id-generator
```

### From Source

```bash
cargo build --release
WORKER_ID=1 ./target/release/id-generator
```

The server listens on `0.0.0.0:8080`.

## ID Format

IDs are 64-bit integers with the following structure:

| Bits | Field | Description |
|------|-------|-------------|
| 41 | Timestamp | Milliseconds since custom epoch |
| 10 | Worker ID | Identifies the generating node |
| 12 | Sequence | Per-millisecond counter (0-4095) |

## License

GNU Affero General Public License v3.0
