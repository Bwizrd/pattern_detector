// src/websocket_server.rs - WebSocket server for real-time clients
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::fmt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use uuid::Uuid;
use dashmap::DashMap;
use log::{info, warn, error, debug};
use tokio::sync::Mutex;

use crate::realtime_monitor::{ZoneEvent, ZoneEventType}; // Import both types
use crate::realtime_monitor::RealTimeZoneMonitor;

#[derive(Debug, Clone)]
pub struct ClientConnection {
    pub id: String,
    pub subscriptions: HashSet<String>, // zone_ids or symbol patterns
    pub sender: mpsc::UnboundedSender<Message>,
}

pub struct WebSocketServer {
    clients: Arc<DashMap<String, ClientConnection>>,
    zone_event_receiver: broadcast::Receiver<ZoneEvent>,
    zone_monitor: Option<Arc<Mutex<RealTimeZoneMonitor>>>,
}

impl WebSocketServer {
    pub fn new(
        zone_event_receiver: broadcast::Receiver<ZoneEvent>,
        zone_monitor: Option<Arc<Mutex<RealTimeZoneMonitor>>>,
    ) -> Self {
        Self {
            clients: Arc::new(DashMap::new()),
            zone_event_receiver,
            zone_monitor,
        }
    }
    
   pub async fn start(&mut self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(addr).await?;
        info!("📡 [WS_SERVER] WebSocket server listening on {}", addr);
        
        // Start event broadcasting task
        let clients_clone = Arc::clone(&self.clients);
        let mut event_receiver = self.zone_event_receiver.resubscribe();
        
        tokio::spawn(async move {
            while let Ok(event) = event_receiver.recv().await {
                Self::broadcast_zone_event(&clients_clone, &event).await;
            }
        });
        
        // Accept connections
        while let Ok((stream, addr)) = listener.accept().await {
            let clients = Arc::clone(&self.clients);
            let zone_monitor = self.zone_monitor.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, addr, clients, zone_monitor).await {
                    error!("❌ [WS_SERVER] Error handling connection from {}: {}", addr, e);
                }
            });
        }
        
         Ok(())
    }
    
    
    async fn handle_connection(
        stream: TcpStream,
        addr: SocketAddr,
        clients: Arc<DashMap<String, ClientConnection>>,
        zone_monitor: Option<Arc<Mutex<RealTimeZoneMonitor>>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ws_stream = accept_async(stream).await?;
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();
        
        let client_id = Uuid::new_v4().to_string();
        let (tx, mut rx) = mpsc::unbounded_channel();
        
        info!("🔗 [WS_SERVER] Client {} connected from {}", client_id, addr);
        
        // Send welcome message
        let welcome = json!({
            "type": "connected",
            "client_id": client_id,
            "message": "Connected to Real-time Zone Monitor",
            "timestamp": chrono::Utc::now().to_rfc3339()
        });
        
        if let Err(e) = ws_sender.send(Message::Text(welcome.to_string())).await {
            error!("❌ [WS_SERVER] Failed to send welcome message: {}", e);
            return Ok(());
        }
        
        // Store client connection
        let client = ClientConnection {
            id: client_id.clone(),
            subscriptions: HashSet::new(),
            sender: tx,
        };
        clients.insert(client_id.clone(), client);
        
        // Spawn task to send messages to client
        let clients_clone = Arc::clone(&clients);
        let client_id_clone = client_id.clone();
        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if let Err(e) = ws_sender.send(message).await {
                    warn!("⚠️  [WS_SERVER] Failed to send message to client {}: {}", client_id_clone, e);
                    clients_clone.remove(&client_id_clone);
                    break;
                }
            }
        });
        
        // Handle incoming messages
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) = Self::handle_client_message(&clients, &client_id, &text, &zone_monitor).await {
                        error!("❌ [WS_SERVER] Error handling message from {}: {}", client_id, e);
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("👋 [WS_SERVER] Client {} disconnected", client_id);
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    if let Some(client) = clients.get(&client_id) {
                        let _ = client.sender.send(Message::Pong(payload));
                    }
                }
                Err(e) => {
                    error!("❌ [WS_SERVER] WebSocket error for client {}: {}", client_id, e);
                    break;
                }
                _ => {}
            }
        }
        
        // Cleanup
        clients.remove(&client_id);
        info!("🧹 [WS_SERVER] Client {} cleaned up", client_id);
        
        Ok(())
    }
    
   async fn handle_client_message(
        clients: &Arc<DashMap<String, ClientConnection>>,
        client_id: &str,
        message: &str,
        zone_monitor: &Option<Arc<Mutex<RealTimeZoneMonitor>>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let request: serde_json::Value = serde_json::from_str(message)?;
        
        match request.get("type").and_then(|t| t.as_str()) {
            Some("get_zones") => {
                if let Some(monitor) = zone_monitor {
                    let monitor_guard = monitor.lock().await;
                    let zones = monitor_guard.get_all_zones().await; // We need to add this method
                    
                    if let Some(client) = clients.get(client_id) {
                        let response = json!({
                            "type": "zones_data",
                            "zones": zones,
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        });
                        
                        let _ = client.sender.send(Message::Text(response.to_string()));
                        debug!("📝 [WS_SERVER] Sent {} zones to client {}", zones.len(), client_id);
                    }
                } else {
                    // No zone monitor available, send empty array
                    if let Some(client) = clients.get(client_id) {
                        let response = json!({
                            "type": "zones_data",
                            "zones": [],
                            "message": "Zone monitor not available",
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        });
                        
                        let _ = client.sender.send(Message::Text(response.to_string()));
                    }
                }
            }
            Some("subscribe_zone") => {
                if let Some(zone_id) = request.get("zone_id").and_then(|z| z.as_str()) {
                    if let Some(mut client) = clients.get_mut(client_id) {
                        client.subscriptions.insert(zone_id.to_string());
                        
                        let response = json!({
                            "type": "subscription_confirmed",
                            "zone_id": zone_id,
                            "message": format!("Subscribed to zone {}", zone_id),
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        });
                        
                        let _ = client.sender.send(Message::Text(response.to_string()));
                        debug!("📝 [WS_SERVER] Client {} subscribed to zone {}", client_id, zone_id);
                    }
                }
            }
            Some("subscribe_symbol") => {
                if let Some(symbol) = request.get("symbol").and_then(|s| s.as_str()) {
                    if let Some(mut client) = clients.get_mut(client_id) {
                        let pattern = format!("symbol:{}", symbol);
                        client.subscriptions.insert(pattern);
                        
                        let response = json!({
                            "type": "subscription_confirmed",
                            "symbol": symbol,
                            "message": format!("Subscribed to all zones for {}", symbol),
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        });
                        
                        let _ = client.sender.send(Message::Text(response.to_string()));
                        debug!("📝 [WS_SERVER] Client {} subscribed to symbol {}", client_id, symbol);
                    }
                }
            }
            Some("subscribe_all") => {
                if let Some(mut client) = clients.get_mut(client_id) {
                    client.subscriptions.insert("*".to_string());
                    
                    let response = json!({
                        "type": "subscription_confirmed",
                        "subscription": "all",
                        "message": "Subscribed to all zone events",
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    });
                    
                    let _ = client.sender.send(Message::Text(response.to_string()));
                    debug!("📝 [WS_SERVER] Client {} subscribed to all events", client_id);
                }
            }
            Some("unsubscribe") => {
                if let Some(subscription) = request.get("subscription").and_then(|s| s.as_str()) {
                    if let Some(mut client) = clients.get_mut(client_id) {
                        client.subscriptions.remove(subscription);
                        
                        let response = json!({
                            "type": "unsubscribe_confirmed",
                            "subscription": subscription,
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        });
                        
                        let _ = client.sender.send(Message::Text(response.to_string()));
                        debug!("📝 [WS_SERVER] Client {} unsubscribed from {}", client_id, subscription);
                    }
                }
            }
            Some("get_stats") => {
                if let Some(client) = clients.get(client_id) {
                    let response = json!({
                        "type": "stats",
                        "connected_clients": clients.len(),
                        "your_subscriptions": client.subscriptions.len(),
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    });
                    
                    let _ = client.sender.send(Message::Text(response.to_string()));
                }
            }
            _ => {
                warn!("❓ [WS_SERVER] Unknown message type from client {}: {}", client_id, message);
            }
        }
        
        Ok(())
    }
    
   async fn broadcast_zone_event(
        clients: &Arc<DashMap<String, ClientConnection>>,
        event: &ZoneEvent,
    ) {
        let message = json!({
            "type": "zone_event",
            "event": event,
            "timestamp": chrono::Utc::now().to_rfc3339()
        });
        
        let message_text = message.to_string();
        let symbol_pattern = format!("symbol:{}", event.symbol);
        
        let mut broadcast_count = 0;
        
        for client in clients.iter() {
            let should_send = client.subscriptions.contains(&event.zone_id) ||
                            client.subscriptions.contains(&symbol_pattern) ||
                            client.subscriptions.contains("*"); // Global subscription
            
            if should_send {
                if let Err(e) = client.sender.send(Message::Text(message_text.clone())) {
                    warn!("⚠️  [WS_SERVER] Failed to send zone event to client {}: {}", client.id, e);
                } else {
                    broadcast_count += 1;
                }
            }
        }
        
        if broadcast_count > 0 {
            debug!("📡 [WS_SERVER] Broadcasted {:?} event to {} clients", event.event_type, broadcast_count);
        }
    }
}