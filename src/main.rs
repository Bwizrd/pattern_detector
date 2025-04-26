// src/main.rs
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use dotenv::dotenv;
use std::env; // Need this for reading env vars

// --- Module Declarations ---
mod backtest;
mod detect;
mod influx_fetcher;
mod patterns;
pub mod trades;
pub mod trading;
mod types;
mod zone_generator;
mod get_zone_by_id;
mod errors;

// --- API & Background Tasking Globals ---
static ZONE_GENERATION_QUEUED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(serde::Serialize)]
struct GeneratorResponse {
    status: String,
    message: String,
}

// --- Web Server Handlers ---
#[derive(serde::Serialize)] struct EchoResponse { data: String, extra: String }
#[derive(serde::Deserialize)] struct QueryParams { pair: String, timeframe: String, range: String, pattern: String }
async fn echo(query: web::Query<QueryParams>) -> impl Responder { /* ... */
     let response = EchoResponse { data: format!("Pair: {}, Timeframe: {}, Range: {}, Pattern: {}", query.pair, query.timeframe, query.range, query.pattern), extra: "Response from Rust".to_string() }; HttpResponse::Ok().json(response)
}
async fn health_check() -> impl Responder { HttpResponse::Ok().body("OK, Rust server is running on port 8080") }

// API handler to queue zone generation
async fn queue_zone_generator_api() -> impl Responder {
    log::info!("Zone generation requested via API");
    ZONE_GENERATION_QUEUED.store(true, std::sync::atomic::Ordering::SeqCst);
    HttpResponse::Accepted().json(GeneratorResponse {
        status: "queued".to_string(),
        message: "Zone generation has been queued. Check server logs for progress.".to_string(),
    })
}

// --- Main Function ---
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    log4rs::init_file("log4rs.yaml", Default::default())
        .expect("Failed to initialize log4rs logging");

    log::info!("Starting Pattern Detector Application...");

    // --- Check if Startup Generation is Enabled ---
    let run_on_startup = env::var("RUN_GENERATOR_ON_STARTUP")
        .map(|val| val.trim().to_lowercase() == "true") // Check if value is "true" (case-insensitive)
        .unwrap_or(false); // Default to false if variable is not set or invalid

    if run_on_startup {
        log::info!("RUN_GENERATOR_ON_STARTUP is true. Running zone generation at startup...");
        // Run synchronously before starting server
        if let Err(e) = zone_generator::run_zone_generation().await {
            log::error!("Startup zone generation failed: {}", e);
            // Decide if this is fatal? For now, just log and continue.
        } else {
            log::info!("Startup zone generation completed successfully.");
        }
    } else {
        log::info!("Skipping zone generation at startup (RUN_GENERATOR_ON_STARTUP is not 'true').");
    }
    // --- End Startup Generation Check ---


    // --- Setup and Start Web Server ---
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port_str = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let port = port_str.parse::<u16>().unwrap_or(8080);

    log::info!("Web server starting on http://{}:{}", host, port);
    println!("---> Starting Pattern Detector Web Server <---");
    println!("---> Listening on: http://{}:{} <---", host, port);
    // ... (println! for endpoints) ...
    println!("Available endpoints:");
    println!("  GET  /analyze?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=...");
    println!("  GET  /active-zones?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=...");
    println!("  POST /bulk-multi-tf-active-zones (Body: [{{symbol, timeframes:[]}}], Query: ?start_time=...&end_time=...&pattern=...)");
    println!("  GET  /debug-bulk-zone?symbol=...&timeframe=...&start_time=...&end_time=...[&pattern=...]");
    println!("  POST /backtest");
    println!("  GET  /testDataRequest");
    println!("  GET  /zone?zone_id=...");
    println!("  GET  /echo?...");
    println!("  GET  /health");
    println!("  POST /generate-zones");
    println!("--------------------------------------------------");

    // --- Background Task Setup (Keep as is) ---
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    {
        let tx_clone = tx.clone(); // Clone tx for the checker task
        tokio::task::spawn_local(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                if ZONE_GENERATION_QUEUED.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    log::info!("Background task detected queued zone generation request via API flag.");
                    if let Err(e) = tx_clone.send(()).await { // Use the cloned tx
                         log::error!("Failed to send signal for queued zone generation: {}", e);
                    }
                }
            }
        });
    }
    // Task to run the generator when signaled
    tokio::task::spawn_local(async move {
        while rx.recv().await.is_some() { // Loop while channel is open
            log::info!("Running queued zone generation task triggered by API...");
            if let Err(e) = zone_generator::run_zone_generation().await {
                log::error!("Queued zone generation failed: {}", e);
            } else {
                log::info!("Queued zone generation completed successfully");
            }
        }
        log::info!("Zone generation listener task finished.");
    });
    // --- End Background Task Setup ---


    // --- Configure and Start HTTP Server ---
    let server = HttpServer::new(move || { // Closure might need move if AppState is added later
        let cors = Cors::default()
            .allowed_origin("http://localhost:4200")
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE"])
            .allowed_headers(vec![
                actix_web::http::header::AUTHORIZATION,
                actix_web::http::header::ACCEPT,
                actix_web::http::header::CONTENT_TYPE,
            ])
            .max_age(3600);
        App::new()
            .wrap(Logger::default())
            .wrap(cors)
            // --- Routes ---
            .route("/analyze", web::get().to(detect::detect_patterns))
            .route("/testDataRequest", web::get().to(influx_fetcher::test_data_request_handler))
            .route("/active-zones", web::get().to(detect::get_active_zones_handler))
            .route("/bulk-multi-tf-active-zones", web::post().to(detect::get_bulk_multi_tf_active_zones_handler))
            .route("/debug-bulk-zone", web::get().to(detect::debug_bulk_zones_handler))
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
            .route("/backtest", web::post().to(backtest::run_backtest))
            .route("/generate-zones", web::post().to(queue_zone_generator_api)) // API trigger remains
            .route("/zone", web::get().to(get_zone_by_id::get_zone_by_id))
    })
    .bind((host, port))?;

    log::info!("Starting Actix HTTP server event loop...");
    server.run().await // Await server termination
}