// src/main.rs
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpServer, HttpResponse, Responder};
use dotenv::dotenv;
use env_logger;

mod detect; // Declares detect module
mod patterns; // Declares patterns module
pub mod trades; 
pub mod trading;
mod backtest;
mod influx_fetcher; 

// Echo endpoint structures
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

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    let host = "127.0.0.1";
    let port = 8080;

    log::info!("Starting server on http://{}:{}", host, port);
    println!("Available endpoints:");
    println!("  GET http://{}:{}/analyze", host, port);
    println!("  GET http://{}:{}/echo", host, port);
    println!("  GET http://{}:{}/testDataRequest", host, port);
    println!("  GET http://{}:{}/health", host, port);
    println!("  GET http://{}:{}/backtest", host, port);
    println!("  GET http://{}:{}/active-zones", host, port);
    log::info!("  POST /backtest");

    // --- Optional: Call the direct test function once on startup ---
    // This runs the fetcher outside of an HTTP request context for comparison
    // You might remove this after initial testing.
    // log::info!("--- Running direct data fetch test on startup ---");
    // influx_fetcher::log_hardcoded_query_results().await;
    // log::info!("--- Direct data fetch test complete ---");
    // --- End Optional direct call ---

    HttpServer::new(|| {
        let cors = Cors::default()
            .allowed_origin("http://localhost:4200")
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE"])
            .allowed_headers(vec![actix_web::http::header::CONTENT_TYPE])
            .max_age(3600);
        App::new()
            .wrap(Logger::default())
            .wrap(cors)
            .route("/analyze", web::get().to(detect::detect_patterns)) // Use detect module
            .route("/testDataRequest", web::get().to(influx_fetcher::test_data_request_handler)) 
            .route("/active-zones", web::get().to(detect::get_active_zones_handler)) 
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
            .route("/backtest", web::post().to(backtest::run_backtest))
    })
    .bind((host, port))?
    .run()
    .await
}