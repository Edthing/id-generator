# unique-id-generator

A Rust web service implementing Twitter's Snowflake ID generation algorithm. Generates unique, time-sorted 64-bit identifiers suitable for distributed systems.

## Features

- Single ID generation endpoint
- Bulk ID generation (up to 4,096,000 IDs per request)
- Handles clock drift and leap seconds
- Thread-safe sequence management

## API Endpoints

### GET /id

Returns a single unique ID.

```json
{"id": "123456789012345678"}
```

### GET /ids/{count}

Returns multiple unique IDs.

```json
{"ids": ["123456789012345678", "123456789012345679", ...]}
```

## Configuration

| Environment Variable | Description | Required |
|---------------------|-------------|----------|
| `WORKER_ID` | Unique worker identifier (0-1023) | Yes |

## Running

### Docker

```bash
docker run -d -p 8080:8080 -e WORKER_ID=1 ghcr.io/Edthing/id-generator
```

### From Source

```bash
cargo build --release
WORKER_ID=1 ./target/release/unique-id-generator
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

AGPL-3.0
