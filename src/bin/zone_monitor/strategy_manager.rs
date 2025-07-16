// src/bin/zone_monitor/strategy_manager.rs
// Strategy management system to fetch and store day trading strategies

use chrono::{DateTime, Utc};
use reqwest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::fs;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    pub id: String,
    pub symbol: String,
    pub r#type: String, // "Day Trading Longs" or "Day Trading Shorts" 
    pub description: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    pub direction: String, // "long" or "short"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategiesFile {
    pub last_updated: DateTime<Utc>,
    pub strategies_by_symbol: HashMap<String, Strategy>,
    pub total_strategies: usize,
}

impl Default for StrategiesFile {
    fn default() -> Self {
        Self {
            last_updated: Utc::now(),
            strategies_by_symbol: HashMap::new(),
            total_strategies: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StrategyManager {
    pub api_url: String,
    pub file_path: String,
    pub http_client: reqwest::Client,
}

impl StrategyManager {
    pub fn new() -> Self {
        let api_url = std::env::var("STRATEGIES_API_URL")
            .unwrap_or_else(|_| "http://localhost:3001/api/strategies".to_string());
        
        let file_path = std::env::var("STRATEGIES_FILE_PATH")
            .unwrap_or_else(|_| "shared_strategies.json".to_string());

        info!("📋 Strategy Manager initialized:");
        info!("   API URL: {}", api_url);
        info!("   File path: {}", file_path);

        Self {
            api_url,
            file_path,
            http_client: reqwest::Client::new(),
        }
    }

    /// Fetch strategies from the API
    pub async fn fetch_strategies(&self) -> Result<Vec<Strategy>, String> {
        info!("📋 Fetching strategies from API: {}", self.api_url);

        match self.http_client
            .get(&self.api_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Vec<Strategy>>().await {
                        Ok(strategies) => {
                            info!("✅ Successfully fetched {} strategies", strategies.len());
                            Ok(strategies)
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to parse strategies JSON: {}", e);
                            error!("❌ {}", error_msg);
                            Err(error_msg)
                        }
                    }
                } else {
                    let error_msg = format!("API returned error status: {}", response.status());
                    error!("❌ {}", error_msg);
                    Err(error_msg)
                }
            }
            Err(e) => {
                let error_msg = format!("Failed to call strategies API: {}", e);
                error!("❌ {}", error_msg);
                Err(error_msg)
            }
        }
    }

    /// Load strategies from the local JSON file
    pub async fn load_strategies_from_file(&self) -> Result<StrategiesFile, String> {
        match fs::read_to_string(&self.file_path).await {
            Ok(content) => {
                match serde_json::from_str::<StrategiesFile>(&content) {
                    Ok(strategies_file) => {
                        Ok(strategies_file)
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to parse strategies file: {}", e);
                        warn!("⚠️ {}", error_msg);
                        Err(error_msg)
                    }
                }
            }
            Err(e) => {
                let error_msg = format!("Failed to read strategies file: {}", e);
                warn!("⚠️ {} - using default", error_msg);
                Ok(StrategiesFile::default())
            }
        }
    }

    /// Save strategies to the local JSON file
    pub async fn save_strategies_to_file(&self, strategies: Vec<Strategy>) -> Result<(), String> {
        // Build strategies by symbol lookup map (one strategy per symbol)
        let mut strategies_by_symbol: HashMap<String, Strategy> = HashMap::new();
        
        for strategy in &strategies {
            strategies_by_symbol.insert(strategy.symbol.clone(), strategy.clone());
        }

        let strategies_file = StrategiesFile {
            last_updated: Utc::now(),
            total_strategies: strategies.len(),
            strategies_by_symbol,
        };

        match serde_json::to_string_pretty(&strategies_file) {
            Ok(json_content) => {
                match fs::write(&self.file_path, json_content).await {
                    Ok(_) => {
                        info!("✅ Saved {} strategies to {}", strategies_file.total_strategies, self.file_path);
                        Ok(())
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to write strategies file: {}", e);
                        error!("❌ {}", error_msg);
                        Err(error_msg)
                    }
                }
            }
            Err(e) => {
                let error_msg = format!("Failed to serialize strategies: {}", e);
                error!("❌ {}", error_msg);
                Err(error_msg)
            }
        }
    }

    /// Fetch strategies from API and save to file
    pub async fn refresh_strategies(&self) -> Result<usize, String> {
        match self.fetch_strategies().await {
            Ok(strategies) => {
                let count = strategies.len();
                match self.save_strategies_to_file(strategies).await {
                    Ok(_) => {
                        info!("🔄 Successfully refreshed {} strategies", count);
                        Ok(count)
                    }
                    Err(e) => {
                        error!("❌ Failed to save refreshed strategies: {}", e);
                        Err(e)
                    }
                }
            }
            Err(e) => {
                error!("❌ Failed to refresh strategies: {}", e);
                Err(e)
            }
        }
    }

    /// Get strategy for a specific symbol
    pub async fn get_strategy_for_symbol(&self, symbol: &str) -> Option<Strategy> {
        match self.load_strategies_from_file().await {
            Ok(strategies_file) => {
                strategies_file
                    .strategies_by_symbol
                    .get(symbol)
                    .cloned()
            }
            Err(_) => None
        }
    }

    /// Check if a symbol has a strategy
    pub async fn has_strategy_for_symbol(&self, symbol: &str) -> bool {
        self.get_strategy_for_symbol(symbol).await.is_some()
    }

    /// Get all strategies
    pub async fn get_all_strategies(&self) -> Vec<Strategy> {
        match self.load_strategies_from_file().await {
            Ok(strategies_file) => strategies_file.strategies_by_symbol.into_values().collect(),
            Err(_) => Vec::new()
        }
    }

    /// Get strategy statistics
    pub async fn get_strategy_stats(&self) -> HashMap<String, serde_json::Value> {
        match self.load_strategies_from_file().await {
            Ok(strategies_file) => {
                let mut stats = HashMap::new();
                
                // Count by direction
                let mut long_count = 0;
                let mut short_count = 0;
                let mut symbol_counts = HashMap::new();
                
                for strategy in strategies_file.strategies_by_symbol.values() {
                    match strategy.direction.as_str() {
                        "long" => long_count += 1,
                        "short" => short_count += 1,
                        _ => {}
                    }
                    
                    *symbol_counts.entry(strategy.symbol.clone()).or_insert(0) += 1;
                }
                
                stats.insert("total_strategies".to_string(), serde_json::Value::Number(strategies_file.total_strategies.into()));
                stats.insert("long_strategies".to_string(), serde_json::Value::Number(long_count.into()));
                stats.insert("short_strategies".to_string(), serde_json::Value::Number(short_count.into()));
                stats.insert("symbols_with_strategies".to_string(), serde_json::Value::Number(symbol_counts.len().into()));
                stats.insert("last_updated".to_string(), serde_json::Value::String(strategies_file.last_updated.to_rfc3339()));
                stats.insert("symbol_counts".to_string(), serde_json::to_value(symbol_counts).unwrap_or_default());
                
                stats
            }
            Err(_) => HashMap::new()
        }
    }
}
