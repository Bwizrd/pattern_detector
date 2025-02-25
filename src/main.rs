// src/main.rs
use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpServer, HttpResponse, Responder};
use dotenv::dotenv;
use env_logger;

mod detect; // Declares detect module
mod patterns; // Declares patterns module

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
    env_logger::init();
    let host = "127.0.0.1";
    let port = 8080;

    println!("Application is running on http://{}:{}", host, port);

    HttpServer::new(|| {
        let cors = Cors::default()
            .allowed_origin("http://localhost:8000")
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE"])
            .allowed_headers(vec![actix_web::http::header::CONTENT_TYPE])
            .max_age(3600);
        App::new()
            .wrap(Logger::default())
            .wrap(cors)
            .route("/analyze", web::get().to(detect::detect_patterns)) // Use detect module
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
    })
    .bind((host, port))?
    .run()
    .await
}