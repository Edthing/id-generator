use actix_web::{web, App, HttpServer, Result, HttpResponse, get, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH, Duration, Instant};
use std::env::var;
use std::sync::Mutex;
use prometheus::{Gauge, IntCounter, Encoder, TextEncoder};
use lazy_static::lazy_static;

lazy_static! {
    static ref IDS_GENERATED: IntCounter = IntCounter::new(
        "id_generator_ids_generated_total", 
        "Total IDs generated"
    ).unwrap();
    
    static ref SEQUENCE_EXHAUSTED: IntCounter = IntCounter::new(
        "id_generator_sequence_exhausted_total",
        "Times sequence was exhausted within a millisecond"
    ).unwrap();
    
    static ref WORKER_ID: Gauge = Gauge::new(
        "id_generator_worker_id",
        "Worker ID of this instance"
    ).unwrap();

    static ref CURRENT_SEQUENCE: Gauge = Gauge::new(
        "id_generator_current_sequence",
        "Current sequence number this ms"
    ).unwrap();
    
    static ref MAX_SEQUENCE_PER_MS: Gauge = Gauge::new(
        "id_generator_max_sequence_per_ms",
        "Maximum sequence number per millisecond"
    ).unwrap();
}

// Constants
const UNIX_EPOCH_OFFSET: u64 = 1705065354064;
const TIMESTAMP_MASK: u64 = 0x1FFFFFFFFFF;
const WORKER_ID_MASK: u64 = 0x3FF;
const SEQUENCE_MASK: u64 = 0xFFF;
const MAX_WORKER_ID: u64 = 1023;

const MAX_IDS_PER_REQUEST: u64 = 4_096_000;
const CLOCK_DRIFT_TIMEOUT_MS: u64 = 100;

// Error response structure for consistent JSON errors
#[derive(Serialize, Deserialize)]
struct ErrorResponse {
    error: String,
}

impl ErrorResponse {
    fn new(message: impl Into<String>) -> Self {
        Self { error: message.into() }
    }
}

// Custom error type for snowflake generation
#[derive(Debug)]
enum SnowflakeError {
    MutexPoisoned,
    ClockDriftTimeout,
}

impl std::fmt::Display for SnowflakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnowflakeError::MutexPoisoned => write!(f, "Internal state error"),
            SnowflakeError::ClockDriftTimeout => write!(f, "Clock drift timeout exceeded"),
        }
    }
}

struct AppState {
    worker_id: u64,
    sequence: Mutex<u64>,
    timestamp: Mutex<u64>,
}

fn get_timestamp() -> u64 {
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH).unwrap_or_default();
    TryInto::<u64>::try_into(since_the_epoch.as_millis()).unwrap_or(0).saturating_sub(UNIX_EPOCH_OFFSET)
}

fn format_snowflake(worker_id: u64, sequence: u64, timestamp: u64) -> u64 {
    ((timestamp & TIMESTAMP_MASK) << 22) | ((worker_id & WORKER_ID_MASK) << 12) | (sequence & SEQUENCE_MASK)
}

fn generate_snowflakes(
    worker_id: u64,
    sequence_mutex: &Mutex<u64>,
    timestamp_mutex: &Mutex<u64>,
    count: u64,
) -> std::result::Result<Vec<String>, SnowflakeError> {
    let mut sequence = sequence_mutex.lock().map_err(|_| SnowflakeError::MutexPoisoned)?;
    let mut last_timestamp = timestamp_mutex.lock().map_err(|_| SnowflakeError::MutexPoisoned)?;
    let mut results = Vec::with_capacity(count as usize);
    let timeout = Duration::from_millis(CLOCK_DRIFT_TIMEOUT_MS);

    for _ in 0..count {
        let mut current_timestamp = get_timestamp();

        // Handle leap seconds / clock drift backwards - wait until time catches up
        if current_timestamp < *last_timestamp {
            let wait_start = Instant::now();
            while current_timestamp < *last_timestamp {
                if wait_start.elapsed() > timeout {
                    return Err(SnowflakeError::ClockDriftTimeout);
                }
                std::thread::sleep(Duration::from_micros(100));
                current_timestamp = get_timestamp();
            }
        }

        if current_timestamp == *last_timestamp {
            *sequence += 1;
            if *sequence > SEQUENCE_MASK {
                SEQUENCE_EXHAUSTED.inc();
                let wait_start = Instant::now();
                while current_timestamp == *last_timestamp {
                    if wait_start.elapsed() > timeout {
                        return Err(SnowflakeError::ClockDriftTimeout);
                    }
                    std::thread::sleep(Duration::from_micros(100));
                    current_timestamp = get_timestamp();
                }
                *sequence = 0;
            }
        } else {
            *sequence = 0;
        }
        *last_timestamp = current_timestamp;
        
        CURRENT_SEQUENCE.set(*sequence as f64);
        IDS_GENERATED.inc();
        
        results.push(format_snowflake(worker_id, *sequence, *last_timestamp).to_string());
    }

    Ok(results)
}

#[derive(Serialize, Deserialize)]
struct Id {
    id: String,
}

#[derive(Serialize, Deserialize)]
struct Bulk {
    ids: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct HealthResponse {
    status: String,
    worker_id: u64,
}

#[get("/health")]
async fn health(data: web::Data<AppState>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(HealthResponse {
        status: "healthy".to_string(),
        worker_id: data.worker_id,
    }))
}

#[get("/metrics")]
async fn metrics() -> Result<HttpResponse> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
         eprintln!("Failed to encode metrics: {}", e);
         return Ok(HttpResponse::InternalServerError().body("Failed to encode metrics"));
    }
    
    match String::from_utf8(buffer) {
        Ok(s) => Ok(HttpResponse::Ok().content_type("text/plain").body(s)),
        Err(e) => {
            eprintln!("Failed to convert metrics to string: {}", e);
            Ok(HttpResponse::InternalServerError().body("Failed to convert metrics to string"))
        }
    }
}

#[get("/id")]
async fn snowflake(data: web::Data<AppState>) -> Result<HttpResponse> {
    match generate_snowflakes(data.worker_id, &data.sequence, &data.timestamp, 1) {
        Ok(ids) => {
            // Safe: we always request 1 ID so the vector is never empty
            let id = ids.into_iter().next().expect("requested 1 ID");
            Ok(HttpResponse::Ok().json(Id { id }))
        }
        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR)
            .json(ErrorResponse::new(e.to_string()))),
    }
}

#[get("/ids/{count}")]
async fn snowflakes(data: web::Data<AppState>, path: web::Path<u64>) -> Result<HttpResponse> {
    let count = path.into_inner();

    if count == 0 {
        return Ok(HttpResponse::BadRequest()
            .json(ErrorResponse::new("Count must be at least 1")));
    }

    if count > MAX_IDS_PER_REQUEST {
        return Ok(HttpResponse::BadRequest()
            .json(ErrorResponse::new(format!(
                "Count must be less than or equal to {}",
                MAX_IDS_PER_REQUEST
            ))));
    }

    match generate_snowflakes(data.worker_id, &data.sequence, &data.timestamp, count) {
        Ok(ids) => Ok(HttpResponse::Ok().json(Bulk { ids })),
        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR)
            .json(ErrorResponse::new(e.to_string()))),
    }
}

fn parse_worker_id() -> std::result::Result<u64, String> {
    // Priority 1: Explicit WORKER_ID environment variable
    if let Ok(worker_id_str) = var("WORKER_ID") {
        let worker_id: u64 = worker_id_str
            .parse()
            .map_err(|_| format!("WORKER_ID must be a valid number, got: '{}'", worker_id_str))?;

        if worker_id > MAX_WORKER_ID {
            return Err(format!(
                "WORKER_ID must be between 0 and {}, got: {}",
                MAX_WORKER_ID, worker_id
            ));
        }
        return Ok(worker_id);
    }

    // Priority 2: Derive from POD_NAME (e.g., "id-generator-0" -> 0)
    if let Ok(pod_name) = var("POD_NAME") {
        if let Some(last_part) = pod_name.rsplit('-').next() {
            if let Ok(id) = last_part.parse::<u64>() {
                if id > MAX_WORKER_ID {
                    return Err(format!(
                        "Derived worker ID from POD_NAME '{}' is {}, which exceeds max {}",
                        pod_name, id, MAX_WORKER_ID
                    ));
                }
                println!("Derived WORKER_ID={} from POD_NAME='{}'", id, pod_name);
                return Ok(id);
            }
        }
    }

    Err("WORKER_ID environment variable is required, or POD_NAME must end with a number".to_string())
}

fn parse_workers() -> u32 {
    var("WORKERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let worker_id = match parse_worker_id() {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Configuration error: {}", e);
            std::process::exit(1);
        }
    };

    let workers = parse_workers();

    println!("Starting id-generator with worker_id={}, workers={}", worker_id, workers);

    // Initialize metrics
    WORKER_ID.set(worker_id as f64);
    MAX_SEQUENCE_PER_MS.set(SEQUENCE_MASK as f64);

    let data = web::Data::new(AppState {
        worker_id,
        sequence: Mutex::new(0),
        timestamp: Mutex::new(0),
    });

    HttpServer::new(move || {
        App::new()
            .app_data(data.clone())
            .service(health)
            .service(metrics)
            .service(snowflake)
            .service(snowflakes)
    })
    .bind(("0.0.0.0", 8080))?
    .workers(workers as usize)
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_snowflake_basic() {
        let id = format_snowflake(0, 0, 0);
        assert_eq!(id, 0);
    }

    #[test]
    fn test_format_snowflake_with_values() {
        // Test with specific values
        let worker_id = 1;
        let sequence = 1;
        let timestamp = 1;

        let id = format_snowflake(worker_id, sequence, timestamp);

        // Verify the bits are in correct positions
        // timestamp (41 bits) << 22 | worker_id (10 bits) << 12 | sequence (12 bits)
        let expected = (1_u64 << 22) | (1_u64 << 12) | 1_u64;
        assert_eq!(id, expected);
    }

    #[test]
    fn test_format_snowflake_max_values() {
        let worker_id = MAX_WORKER_ID;
        let sequence = SEQUENCE_MASK;
        let timestamp = TIMESTAMP_MASK;

        let id = format_snowflake(worker_id, sequence, timestamp);

        // Extract and verify each component
        let extracted_sequence = id & SEQUENCE_MASK;
        let extracted_worker = (id >> 12) & WORKER_ID_MASK;
        let extracted_timestamp = (id >> 22) & TIMESTAMP_MASK;

        assert_eq!(extracted_sequence, SEQUENCE_MASK);
        assert_eq!(extracted_worker, MAX_WORKER_ID);
        assert_eq!(extracted_timestamp, TIMESTAMP_MASK);
    }

    #[test]
    fn test_format_snowflake_masks_overflow() {
        // Values exceeding mask should be truncated
        let worker_id = 2048; // > 1023
        let sequence = 8192;  // > 4095

        let id = format_snowflake(worker_id, sequence, 0);

        let extracted_worker = (id >> 12) & WORKER_ID_MASK;
        let extracted_sequence = id & SEQUENCE_MASK;

        // Should be masked to valid ranges
        assert_eq!(extracted_worker, worker_id & WORKER_ID_MASK);
        assert_eq!(extracted_sequence, sequence & SEQUENCE_MASK);
    }

    #[test]
    fn test_get_timestamp_returns_positive() {
        let ts = get_timestamp();
        // After UNIX_EPOCH_OFFSET (Jan 2024), timestamp should be positive
        assert!(ts > 0, "Timestamp should be positive after epoch offset");
    }

    #[test]
    fn test_get_timestamp_increases() {
        let ts1 = get_timestamp();
        std::thread::sleep(Duration::from_millis(2));
        let ts2 = get_timestamp();
        assert!(ts2 >= ts1, "Timestamp should not decrease");
    }

    #[test]
    fn test_generate_snowflakes_single() {
        let sequence = Mutex::new(0);
        let timestamp = Mutex::new(0);

        let result = generate_snowflakes(1, &sequence, &timestamp, 1);

        assert!(result.is_ok());
        let ids = result.unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids[0].parse::<u64>().is_ok());
    }

    #[test]
    fn test_generate_snowflakes_multiple() {
        let sequence = Mutex::new(0);
        let timestamp = Mutex::new(0);

        let result = generate_snowflakes(1, &sequence, &timestamp, 100);

        assert!(result.is_ok());
        let ids = result.unwrap();
        assert_eq!(ids.len(), 100);
    }

    #[test]
    fn test_generate_snowflakes_uniqueness() {
        let sequence = Mutex::new(0);
        let timestamp = Mutex::new(0);

        let result = generate_snowflakes(1, &sequence, &timestamp, 1000);

        assert!(result.is_ok());
        let ids = result.unwrap();

        // Check all IDs are unique
        let mut unique_ids: std::collections::HashSet<&String> = std::collections::HashSet::new();
        for id in &ids {
            assert!(unique_ids.insert(id), "Duplicate ID found: {}", id);
        }
    }

    #[test]
    fn test_generate_snowflakes_ordering() {
        let sequence = Mutex::new(0);
        let timestamp = Mutex::new(0);

        let result = generate_snowflakes(1, &sequence, &timestamp, 100);

        assert!(result.is_ok());
        let ids: Vec<u64> = result.unwrap()
            .iter()
            .map(|s| s.parse::<u64>().unwrap())
            .collect();

        // IDs should be monotonically increasing
        for i in 1..ids.len() {
            assert!(ids[i] > ids[i-1], "IDs should be monotonically increasing");
        }
    }

    #[test]
    fn test_generate_snowflakes_worker_id_embedded() {
        let sequence = Mutex::new(0);
        let timestamp = Mutex::new(0);
        let worker_id = 42;

        let result = generate_snowflakes(worker_id, &sequence, &timestamp, 10);

        assert!(result.is_ok());
        for id_str in result.unwrap() {
            let id: u64 = id_str.parse().unwrap();
            let extracted_worker = (id >> 12) & WORKER_ID_MASK;
            assert_eq!(extracted_worker, worker_id, "Worker ID should be embedded in snowflake");
        }
    }

    #[test]
    fn test_parse_worker_id_missing() {
        // This test relies on WORKER_ID not being set
        std::env::remove_var("WORKER_ID");
        let result = parse_worker_id();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("required"));
    }

    #[test]
    fn test_parse_worker_id_invalid() {
        std::env::set_var("WORKER_ID", "not_a_number");
        let result = parse_worker_id();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("valid number"));
        std::env::remove_var("WORKER_ID");
    }

    #[test]
    fn test_parse_worker_id_out_of_range() {
        std::env::set_var("WORKER_ID", "1024");
        let result = parse_worker_id();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("between 0 and 1023"));
        std::env::remove_var("WORKER_ID");
    }

    #[test]
    fn test_parse_worker_id_valid() {
        std::env::set_var("WORKER_ID", "512");
        let result = parse_worker_id();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 512);
        std::env::remove_var("WORKER_ID");
    }

    #[test]
    fn test_parse_worker_id_boundary_zero() {
        std::env::set_var("WORKER_ID", "0");
        let result = parse_worker_id();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
        std::env::remove_var("WORKER_ID");
    }

    #[test]
    fn test_parse_worker_id_boundary_max() {
        std::env::set_var("WORKER_ID", "1023");
        let result = parse_worker_id();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1023);
        std::env::remove_var("WORKER_ID");
    }

    #[test]
    fn test_parse_workers_default() {
        std::env::remove_var("WORKERS");
        let workers = parse_workers();
        assert_eq!(workers, 1);
    }

    #[test]
    fn test_parse_workers_custom() {
        std::env::set_var("WORKERS", "4");
        let workers = parse_workers();
        assert_eq!(workers, 4);
        std::env::remove_var("WORKERS");
    }

    #[test]
    fn test_parse_workers_invalid_falls_back() {
        std::env::set_var("WORKERS", "invalid");
        let workers = parse_workers();
        assert_eq!(workers, 1);
        std::env::remove_var("WORKERS");
    }

    #[test]
    fn test_error_response_new() {
        let err = ErrorResponse::new("test error");
        assert_eq!(err.error, "test error");
    }

    #[test]
    fn test_snowflake_error_display() {
        assert_eq!(
            SnowflakeError::MutexPoisoned.to_string(),
            "Internal state error"
        );
        assert_eq!(
            SnowflakeError::ClockDriftTimeout.to_string(),
            "Clock drift timeout exceeded"
        );
    }

    #[test]
    fn test_sequence_overflow_handling() {
        let sequence = Mutex::new(SEQUENCE_MASK); // Start at max
        let timestamp = Mutex::new(0);

        // This should trigger sequence overflow handling
        let result = generate_snowflakes(1, &sequence, &timestamp, 2);

        assert!(result.is_ok());
        let ids = result.unwrap();
        assert_eq!(ids.len(), 2);

        // Both should be valid and unique
        assert_ne!(ids[0], ids[1]);
    }
}
