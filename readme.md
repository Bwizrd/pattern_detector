tree -I "reports|target|.git|node_modules"

cargo test --test backtest -- --nocapture
cargo test --test multithreaded_optimization -- --nocapture
cargo test --test optimization/main -- --nocapture

I am building a forex data pattern recognizer in rust and displaying the results in an Angular App using SciCharts. The Data is stored in an Influx db.

The Rust backend has 

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::init();
    let host = "127.0.0.1";
    let port = 8080;

    println!("Application is running on http://{}:{}", host, port);
    println!("Available endpoints:");
    println!("  GET http://{}:{}/analyze", host, port);
    println!("  GET http://{}:{}/echo", host, port);
    println!("  GET http://{}:{}/health", host, port);

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
            .route("/echo", web::get().to(echo))
            .route("/health", web::get().to(health_check))
    })
    .bind((host, port))?
    .run()
    .await
}

in its main.rs. /analyze runs detect.rs

Please assimilate this and wait for further instruction.


On the Angular side we have a PatterRegistrySerivce that allows me to register patterns.

The next thing I want to work on in my App is a Supply and Demand Zone Detection over multiple currencies and timeframes.




Here is an example of a Rust pattern detector. I want to create one for 50% body candles




Now I'm wondering if I could use recursion to go through all the currency pairs and all the timeframes and then keep trying multiple stop loss, tp and lot sizes to find the optimal profit factor across all of them. Rust is good at concurrency so this should be super fast too. Is that possible? And to also have a report created.