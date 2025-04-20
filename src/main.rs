// src/main.rs
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use dotenv::dotenv;

mod backtest;
mod detect;
mod influx_fetcher;
mod patterns;
pub mod trades;
pub mod trading;
mod types; // types.rs has been updated
mod zone_generator; // Add this line to include the zone_generator module

static ZONE_GENERATION_QUEUED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(serde::Serialize)]
struct EchoResponse {
    data: String,
    extra: String,
}

#[derive(serde::Deserialize)]
struct QueryParams {
    pair: String,
    timeframe: String,
    range: String,
    pattern: String,
}

#[derive(serde::Serialize)]
struct GeneratorResponse {
    status: String,
    message: String,
}

async fn echo(query: web::Query<QueryParams>) -> impl Responder {
    let response = EchoResponse {
        data: format!(
            "Pair: {}, Timeframe: {}, Range: {}, Pattern: {}",
            query.pair, query.timeframe, query.range, query.pattern
        ),
        extra: "Response from Rust".to_string(),
    };
    HttpResponse::Ok().json(response)
}

async fn health_check() -> impl Responder {
    HttpResponse::Ok().body("OK, Rust server is running on port 8080")
}

// New handler to run zone generation
async fn run_zone_generator() -> impl Responder {
    log::info!("Zone generation requested via API");
    
    // Queue the operation rather than running it directly
    ZONE_GENERATION_QUEUED.store(true, std::sync::atomic::Ordering::SeqCst);
    
    // Immediately respond that the process has been queued
    HttpResponse::Accepted().json(GeneratorResponse {
        status: "queued".to_string(),
        message: "Zone generation has been queued. Check server logs for progress.".to_string(),
    })
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    log4rs::init_file("log4rs.yaml", Default::default()).expect("Failed to initialize log4rs");
    let host = "127.0.0.1";
    let port = 8080;

    // Run zone generation at startup
    // Uncomment the next block to run zone generation when the application starts
    {
        log::info!("Running zone generation at startup...");
        // Don't use tokio::spawn here
        if let Err(e) = zone_generator::run_zone_generation().await {
            log::error!("Startup zone generation failed: {}", e);
        } else {
            log::info!("Startup zone generation completed successfully");
        }
    }

    log::info!("Starting server on http://{}:{}", host, port);
    println!("Available endpoints:");
    println!("  GET http://{}:{}/analyze?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=...", host, port);
    println!("  GET http://{}:{}/active-zones?symbol=...&timeframe=...&start_time=...&end_time=...&pattern=...", host, port);
    println!("  POST http://{}:{}/bulk-multi-tf-active-zones (Body: [{{symbol, timeframes:[]}}], Query: ?start_time=...&end_time=...&pattern=...)", host, port);
    println!("  GET http://{}:{}/debug-bulk-zone?symbol=...&timeframe=...&start_time=...&end_time=...[&pattern=...]", host, port);
    println!("  POST http://{}:{}/backtest (Body: BacktestRequest)", host, port);
    println!("  GET http://{}:{}/testDataRequest", host, port);
    println!("  GET http://{}:{}/echo?pair=...&timeframe=...&range=...&pattern=...", host, port);
    println!("  GET http://{}:{}/health", host, port);
    println!("  POST http://{}:{}/generate-zones", host, port); // Keep the endpoint for manual triggering

    // Set up a background task to check for queued zone generation requests
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    
    // Start a task that periodically checks if zone generation is requested
    {
        let tx = tx.clone();
        tokio::task::spawn_local(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                if ZONE_GENERATION_QUEUED.load(std::sync::atomic::Ordering::SeqCst) {
                    // Reset the flag first to avoid multiple executions
                    ZONE_GENERATION_QUEUED.store(false, std::sync::atomic::Ordering::SeqCst);
                    log::info!("Background task detected queued zone generation request");
                    
                    // Send a message to the main task to run zone generation
                    let _ = tx.send(()).await;
                }
            }
        });
    }

    // Create the HTTP server
    let server = HttpServer::new(|| {
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
            .route("/analyze", web::get().to(detect::detect_patterns))
            .route(
                "/testDataRequest",
                web::get().to(influx_fetcher::test_data_request_handler),
            )
            .route(
                "/active-zones",
                web::get().to(detect::get_active_zones_handler),
            )
            .route(
                "/bulk-multi-tf-active-zones",
                web::post().to(detect::get_bulk_multi_tf_active_zones_handler),
            )
            .route(
                "/debug-bulk-zone",
                web::get().to(detect::debug_bulk_zones_handler),
            )
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
            .route("/backtest", web::post().to(backtest::run_backtest))
            .route("/generate-zones", web::post().to(run_zone_generator)) // Route for zone generation
    })
    .bind((host, port))?;
    
    // Start the server
    let server_handle = server.run();
    
    // Process zone generation requests in the main task
    tokio::task::spawn_local(async move {
        while let Some(_) = rx.recv().await {
            log::info!("Running queued zone generation task");
            if let Err(e) = zone_generator::run_zone_generation().await {
                log::error!("Queued zone generation failed: {}", e);
            } else {
                log::info!("Queued zone generation completed successfully");
            }
        }
    });
    
    // Wait for the server to complete
    server_handle.await
}