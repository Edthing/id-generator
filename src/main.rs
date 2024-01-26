use actix_web::rt::task::yield_now;
use actix_web::{web, App, HttpServer, Result, HttpResponse, get};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use std::env::var;
use std::sync::{Mutex, MutexGuard};

// Constants
const UNIX_EPOCH_OFFSET: u64 = 1705065354064;
const TIMESTAMP_MASK: u64 = 0x1FFFFFFFFFF;
const WORKER_ID_MASK: u64 = 0x3FF;
const SEQUENCE_MASK: u64 = 0xFFF;

const MAX_IDS_PER_REQUEST: u64 = 4_096_000; // equal to a single workers throughput per second

struct AppState {
    worker_id: u64,
    sequence: Mutex<u64>,
    timestamp: Mutex<u64>,
}

fn get_timestamp() -> u64 {
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH).expect("Time went backwards");
    return TryInto::<u64>::try_into(since_the_epoch.as_millis()).unwrap() - UNIX_EPOCH_OFFSET;
}

fn format_snowflake(worker_id: u64, sequence: u64, timestamp: u64) -> u64 {
    return ((timestamp & TIMESTAMP_MASK) << 22) | ((worker_id & WORKER_ID_MASK) << 12) | (sequence & SEQUENCE_MASK);
}

fn generate_snowflake(worker_id: u64, mut sequence: MutexGuard<u64>, mut timestamp: MutexGuard<u64>) -> u64 {
    let mut current_timestamp = get_timestamp();
    if current_timestamp == *timestamp {
        *sequence += 1;
        if *sequence > SEQUENCE_MASK {
            while current_timestamp == *timestamp {
                current_timestamp = get_timestamp();
            }
            *sequence = 0;
        }
    } else {
        *sequence = 0;
    }
    *timestamp = current_timestamp;
    return format_snowflake(worker_id, *sequence, *timestamp);
}

#[derive(Serialize, Deserialize)]
struct Id {
    id: String,
}

#[get("/id")]
async fn snowflake(data: web::Data<AppState>) -> Result<HttpResponse> {
    let flake = generate_snowflake(data.worker_id, data.sequence.lock().unwrap(), data.timestamp.lock().unwrap());
    let flake_str = flake.to_string();
    Ok(HttpResponse::Ok().json(Id { id: flake_str }))
}

// bulk endpoint
#[derive(Serialize, Deserialize)]
struct Bulk {
    ids: Vec<String>,
}

#[get("/ids/{count}")]
async fn snowflakes(data: web::Data<AppState>, path: web::Path<u64>) -> Result<HttpResponse> {
    let count = path.into_inner();
    if count > MAX_IDS_PER_REQUEST {
        return Ok(HttpResponse::BadRequest().body("Count must be less than or equal to ".to_owned() + &MAX_IDS_PER_REQUEST.to_string()));
    }
    let mut snowflakes: Vec<String> = Vec::new();
    for _ in 0..count {
        let flake = generate_snowflake(data.worker_id, data.sequence.lock().unwrap(), data.timestamp.lock().unwrap());
        let flake_str = flake.to_string();
        snowflakes.push(flake_str);
        yield_now().await;
    }
    Ok(HttpResponse::Ok().json(Bulk { ids: snowflakes }))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let worker_id: u64 = var("WORKER_ID").unwrap().parse::<u64>().unwrap();
    let data = web::Data::new(AppState {
        worker_id: worker_id,
        sequence: Mutex::new(0),
        timestamp: Mutex::new(0),
    });
    HttpServer::new(move || {
        App::new()
            .app_data(data.clone())
            .service(snowflake)
            .service(snowflakes)
    })
    .bind(("0.0.0.0", 8080))?
    .workers(1)
    .run()
    .await
}
