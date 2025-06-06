// src/errors.rs
use actix_web::{ResponseError, HttpResponse};
use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Environment variable error: {0}")]
    EnvVarError(#[from] std::env::VarError),

    #[error("Reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("CSV parsing error: {0}")]
    CsvError(#[from] csv::Error),

    #[error("Data parsing error: {0}")]
    ParseError(String),

    #[error("InfluxDB query failed: {0}")]
    InfluxQueryError(String),

    #[error("Zone not found")]
    NotFound,

    #[error("Internal Server Error")]
    InternalError,
}

impl ResponseError for ServiceError {
    fn error_response(&self) -> HttpResponse {
        match self {
            ServiceError::NotFound => HttpResponse::NotFound().finish(),
            ServiceError::EnvVarError(_) => {
                log::error!("Configuration error: {}", self);
                HttpResponse::InternalServerError().body("Server configuration error")
            }
            ServiceError::ReqwestError(e) => {
                 log::error!("HTTP client error: {}", e);
                 HttpResponse::InternalServerError().body("Error communicating with database")
            }
            ServiceError::CsvError(e) => {
                 log::error!("CSV parsing error: {}", e);
                 HttpResponse::InternalServerError().body("Error processing database response")
            }
             ServiceError::ParseError(msg) => {
                 log::error!("Data parsing error: {}", msg);
                 HttpResponse::InternalServerError().body("Error processing data")
            }
             ServiceError::InfluxQueryError(msg) => {
                 log::error!("InfluxDB query failed: {}", msg);
                 HttpResponse::InternalServerError().body("Database query execution failed")
            }
            ServiceError::InternalError => HttpResponse::InternalServerError().finish(),
        }
    }
}