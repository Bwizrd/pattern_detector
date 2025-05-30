// src/simple_price_websocket.rs
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use log::{error, info};
use serde_json::json;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};

pub struct SimplePriceWebSocketServer {
    clients: Arc<DashMap<String, tokio::sync::mpsc::UnboundedSender<Message>>>,
    price_broadcaster: tokio::sync::broadcast::Sender<String>,
}

impl SimplePriceWebSocketServer {
    pub fn new() -> (Self, tokio::sync::broadcast::Sender<String>) {
        let (price_broadcaster, _) = tokio::sync::broadcast::channel::<String>(1000);
        let server = Self {
            clients: Arc::new(DashMap::new()),
            price_broadcaster: price_broadcaster.clone(),
        };
        (server, price_broadcaster)
    }

    pub async fn start(
        &self,
        addr: String,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(&addr).await?;
        info!("📡 [SIMPLE_WS] Listening on {}", addr);

        // Start price broadcaster task
        let clients_clone = Arc::clone(&self.clients);
        let mut price_receiver = self.price_broadcaster.subscribe();
        tokio::spawn(async move {
            while let Ok(price_data) = price_receiver.recv().await {
                let message = Message::Text(price_data);
                clients_clone.retain(|_, sender| sender.send(message.clone()).is_ok());
            }
        });

        // Accept client connections
        while let Ok((stream, addr)) = listener.accept().await {
            let clients = Arc::clone(&self.clients);
            tokio::spawn(Self::handle_client(stream, addr, clients));
        }

        while let Ok((stream, client_addr)) = listener.accept().await {
            info!("🔗 [SIMPLE_WS] Incoming connection from {}", client_addr);
            let clients = Arc::clone(&self.clients);
            tokio::spawn(Self::handle_client(stream, client_addr, clients));
        }

        Ok(())
    }

    async fn handle_client(
        stream: TcpStream,
        addr: std::net::SocketAddr,
        clients: Arc<DashMap<String, tokio::sync::mpsc::UnboundedSender<Message>>>,
    ) {
        let client_id = uuid::Uuid::new_v4().to_string();
        info!(
            "🔗 [SIMPLE_WS] Client {} connected from {}",
            client_id, addr
        );
        info!("🔧 [SIMPLE_WS] Starting to handle client from {}", addr);
        let client_id = uuid::Uuid::new_v4().to_string();
        info!(
            "🔗 [SIMPLE_WS] Client {} connected from {}",
            client_id, addr
        );

        if let Ok(ws_stream) = accept_async(stream).await {
            let (mut ws_sender, mut ws_receiver) = ws_stream.split();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

            // Store client
            clients.insert(client_id.clone(), tx);

            // Send welcome message
            let welcome = json!({
                "type": "connected",
                "client_id": client_id,
                "message": "Connected to simple price feed"
            });
            let _ = ws_sender.send(Message::Text(welcome.to_string())).await;

            // Handle outgoing messages
            let client_id_clone = client_id.clone();
            let clients_clone = Arc::clone(&clients);
            tokio::spawn(async move {
                while let Some(message) = rx.recv().await {
                    if ws_sender.send(message).await.is_err() {
                        clients_clone.remove(&client_id_clone);
                        break;
                    }
                }
            });

            // Handle incoming messages (simple echo)
            while let Some(msg) = ws_receiver.next().await {
                if msg.is_err() {
                    break;
                }
            }
        }

        clients.remove(&client_id);
        info!("👋 [SIMPLE_WS] Client {} disconnected", client_id);
    }
}

pub fn broadcast_price_update(broadcaster: &tokio::sync::broadcast::Sender<String>, message: &str) {
    let price_message = json!({
        "type": "raw_price_update",
        "data": serde_json::from_str::<serde_json::Value>(message).unwrap_or_default(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    let _ = broadcaster.send(price_message.to_string());
}
