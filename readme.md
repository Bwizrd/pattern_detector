tree -I "reports|target|.git|node_modules"

tree -I "reports|target|.git|logs|web|trades|test_scripts|tests"
cd usr/local/var/lib/influxdb2 influxd

cargo test --test backtest -- --nocapture
cargo test --test multithreaded_optimization -- --nocapture
cargo test --test optimization/main -- --nocapture

cargo test --test optimization multi_currency::run_multi_currency_optimization -- --nocapture

cargo test --test optimization advanced_optimizer::run_advanced_optimization -- --nocapture
RUST_LOG=debug,pattern_detector::trading=debug cargo test --test optimization advanced_optimizer::run_advanced_optimization -- --nocapture
RUST_LOG=info,optimization::advanced_optimizer=debug,pattern_detector::trading=info cargo test --test optimization advanced_optimizer::run_advanced_optimization -- --nocapture

RUST_LOG=info,optimization::performance_scanner=debug cargo test --test optimization performance_scanner::run_performance_scan -- --nocapture

I am building a forex data pattern recognizer in rust and displaying the results in an Angular App using SciCharts. The Data is stored in an Influx db.

The Rust backend has 

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("pattern_detector=debug,info"));
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

in its main.rs. /active-zones runs detect.rs

pub async fn get_active_zones_handler(query: web::Query<ChartQuery>) -> impl Responder {
     info!("[get_active_zones] Handling /active-zones request: {:?}", query);
     match _fetch_and_detect_core(&query).await {
         // Receive candles and RAW detection_results. Ignore recognizer.
        Ok((candles, _recognizer, mut detection_results)) => { // Make results mutable

             // --- Handle No Candles Case ---
            if candles.is_empty() {
                 info!("[get_active_zones] No candle data found by core logic. Returning empty structure.");
                 // Return the specific structure desired for this endpoint when no data
                 return HttpResponse::Ok().json(json!({
                     "pattern": query.pattern,
                     "status": "All Zones with Activity Status and Candles",
                     "time_range": { "start": query.start_time, "end": query.end_time },
                     "symbol": query.symbol,
                     "timeframe": query.timeframe,
                     "supply_zones": [],
                     "demand_zones": [],
                     "message": "No candle data found for the specified parameters."
                 }));
             }

             info!("[get_active_zones] Enriching detection results ({} candles)...", candles.len());
             // --- Enrichment Logic (Modifies detection_results in place) ---
            let process_zones = |zones_option: Option<&mut Vec<Value>>, is_supply: bool| {
                 if let Some(zones) = zones_option {
                     info!("[get_active_zones] Enriching {} {} zones.", zones.len(), if is_supply {"supply"} else {"demand"});
                     for zone_json in zones.iter_mut() { // Iterate mutably
                         let is_active = is_zone_still_active(zone_json, &candles, is_supply);
                         if let Some(obj) = zone_json.as_object_mut() {
                             obj.insert("is_active".to_string(), json!(is_active));
                             info!("[get_active_zones] Inserted is_active={} for zone starting at {:?}", is_active, obj.get("start_idx"));
                             if let Some(start_idx) = obj.get("start_idx").and_then(|v| v.as_u64()) {
                                 let zone_candles = candles.get(start_idx as usize ..).unwrap_or(&[]).to_vec();
                                 obj.insert("candles".to_string(), json!(zone_candles));
                             } // Handle missing start_idx if needed
                         }
                     }
                 } // Handle case where zones_option is None if structure might vary
            };

            // Get mutable access and call enrichment
             if let Some(data) = detection_results.get_mut("data").and_then(|d| d.get_mut("price")) {
                 process_zones(data.get_mut("supply_zones").and_then(|sz| sz.get_mut("zones")).and_then(|z| z.as_array_mut()), true);
                 process_zones(data.get_mut("demand_zones").and_then(|dz| dz.get_mut("zones")).and_then(|z| z.as_array_mut()), false);
             } else { warn!("[get_active_zones] Could not find expected structure 'data.price...zones' to enrich."); }


            info!("[get_active_zones] Enrichment complete.");

        //     // --- ADD LOGGING BLOCK ---
        //     match serde_json::to_string_pretty(&detection_results) {
        //         Ok(json_string) => {
        //             info!("[get_active_zones] Final JSON structure to be sent (pretty):\n{}", json_string);
        //         }
        //         Err(e) => {
        //             error!("[get_active_zones] Failed to serialize final JSON for logging: {}", e);
        //         }
        //    }
           // --- END LOGGING BLOCK ---

             // Return the modified detection_results
             HttpResponse::Ok().json(detection_results)
        }
        Err(e) => {
             error!("[get_active_zones] Core error: {}", e);
             // Map CoreError to appropriate HTTP responses (can be same as detect_patterns)
             match e {
                 CoreError::Config(msg) if msg.starts_with("Unknown pattern") => HttpResponse::BadRequest().body(msg),
                 CoreError::Config(msg) if msg == "CSV header mismatch" => HttpResponse::InternalServerError().body("CSV header mismatch error"),
                  // Add other specific mappings as needed, matching detect_patterns where appropriate
                 _ => HttpResponse::InternalServerError().body(format!("Internal processing error: {}", e))
             }
         }
     }
}

This endpoint works for one forex pair and timeframe. I want to create an endpoint that detects active zones for all supplied forex pairs and timeframes and them in a table in the Angular frontend



Please assimilate this and wait for further instruction.


On the Angular side we have a PatterRegistrySerivce that allows me to register patterns.

The next thing I want to work on in my App is a Supply and Demand Zone Detection over multiple currencies and timeframes.




Here is an example of a Rust pattern detector. I want to create one for 50% body candles




Now I'm wondering if I could use recursion to go through all the currency pairs and all the timeframes and then keep trying multiple stop loss, tp and lot sizes to find the optimal profit factor across all of them. Rust is good at concurrency so this should be super fast too. Is that possible? And to also have a report created.