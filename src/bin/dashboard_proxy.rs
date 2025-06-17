// src/bin/dashboard_proxy.rs - Secure proxy for zrok that only exposes dashboard endpoints

use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Result};
use actix_cors::Cors;
use reqwest::Client;
use std::collections::HashSet;

// Only allow these specific paths for security
fn get_allowed_paths() -> HashSet<&'static str> {
    let mut paths = HashSet::new();
    paths.insert("/");
    paths.insert("/dashboard");
    paths.insert("/api/dashboard-data");
    paths
}

async fn proxy_handler(
    req: HttpRequest,
    body: web::Bytes,
    client: web::Data<Client>,
) -> Result<HttpResponse> {
    let path = req.uri().path();
    let allowed_paths = get_allowed_paths();
    
    // Redirect root to dashboard
    if path == "/" {
        return Ok(HttpResponse::Found()
            .insert_header(("Location", "/dashboard"))
            .finish());
    }
    
    // Check if path is allowed
    if !allowed_paths.contains(path) {
        println!("🚫 [PROXY] Blocked access to: {}", path);
        return Ok(HttpResponse::NotFound()
            .body("404 - Only dashboard access allowed"));
    }
    
    // Build backend URL
    let backend_url = format!("http://localhost:8080{}", req.uri());
    
    println!("✅ [PROXY] Forwarding: {} -> {}", path, backend_url);
    
    // Create request to backend
    let mut backend_req = match req.method().as_str() {
        "GET" => client.get(&backend_url),
        "POST" => client.post(&backend_url),
        "PUT" => client.put(&backend_url),
        "DELETE" => client.delete(&backend_url),
        _ => {
            return Ok(HttpResponse::MethodNotAllowed()
                .body("Method not allowed"));
        }
    };
    
    // Copy relevant headers
    for (name, value) in req.headers() {
        if let Ok(name_str) = name.as_str().parse::<reqwest::header::HeaderName>() {
            if name_str != "host" && name_str != "content-length" {
                if let Ok(value_str) = value.to_str() {
                    backend_req = backend_req.header(name_str, value_str);
                }
            }
        }
    }
    
    // Add body if present
    if !body.is_empty() {
        backend_req = backend_req.body(body.to_vec());
    }
    
    // Send request to backend
    match backend_req.send().await {
        Ok(backend_resp) => {
            let status = backend_resp.status();
            let headers = backend_resp.headers().clone();
            let body = backend_resp.bytes().await.unwrap_or_default();
            
            let mut response = HttpResponse::build(
                actix_web::http::StatusCode::from_u16(status.as_u16()).unwrap()
            );
            
            // Copy response headers
            for (name, value) in headers {
                if let (Some(name), Ok(value_str)) = (name, value.to_str()) {
                    if name != "server" && name != "date" {
                        response.insert_header((name.as_str(), value_str));
                    }
                }
            }
            
            Ok(response.body(body))
        }
        Err(e) => {
            println!("❌ [PROXY] Backend error: {}", e);
            Ok(HttpResponse::ServiceUnavailable()
                .body(format!("Backend unavailable: {}", e)))
        }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("🚀 Starting Dashboard Proxy for zrok");
    println!("📱 Only exposing dashboard endpoints:");
    for path in get_allowed_paths() {
        println!("   ✅ {}", path);
    }
    println!("🔒 All other endpoints blocked for security");
    println!();
    
    let client = Client::new();
    let port = std::env::var("PROXY_PORT")
        .unwrap_or_else(|_| "3001".to_string())
        .parse::<u16>()
        .unwrap_or(3001);
    
    println!("📡 Dashboard Proxy starting on http://localhost:{}", port);
    println!("🌐 Ready for zrok private share!");
    
    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();
            
        App::new()
            .wrap(cors)
            .app_data(web::Data::new(client.clone()))
            .default_service(web::route().to(proxy_handler))
    })
    .bind(("127.0.0.1", port))?
    .run()
    .await
}