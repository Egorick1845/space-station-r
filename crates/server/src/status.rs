//! HTTP-статус сервера для лаунчера (`StatusHost` SS14): `GET /status` на
//! том же номере порта по TCP. Отдельный поток слушает сокет и отвечает
//! снимком, который система Bevy обновляет раз в секунду.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use ssr_protocol::status::{ServerStatus, http_json_response};

/// Снимок статуса, общий с потоком HTTP.
#[derive(Resource, Clone)]
pub struct StatusSnapshot(pub Arc<Mutex<ServerStatus>>);

/// Название сервера (`SSR_SERVER_NAME`) и мягкий лимит (`SSR_MAX_PLAYERS`).
fn initial_status() -> ServerStatus {
    ServerStatus {
        name: std::env::var("SSR_SERVER_NAME")
            .unwrap_or_else(|_| "Space Station R — локальный сервер".to_string()),
        players: 0,
        soft_max_players: std::env::var("SSR_MAX_PLAYERS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(64),
        round_time_secs: 0,
        protocol_version: ssr_protocol::PROTOCOL_VERSION,
    }
}

/// Запускает поток HTTP-статуса и ставит ресурс снимка.
pub fn start(app: &mut App, addr: SocketAddr) {
    let snapshot = StatusSnapshot(Arc::new(Mutex::new(initial_status())));
    let shared = snapshot.0.clone();
    match TcpListener::bind(addr) {
        Ok(listener) => {
            tracing::info!(%addr, "status: HTTP /status слушает");
            std::thread::spawn(move || serve(listener, shared));
        }
        Err(error) => tracing::warn!(%addr, %error, "status: не удалось открыть порт"),
    }
    app.insert_resource(snapshot);
}

fn serve(listener: TcpListener, shared: Arc<Mutex<ServerStatus>>) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut buffer = [0u8; 1024];
        let Ok(read) = stream.read(&mut buffer) else {
            continue;
        };
        let request = String::from_utf8_lossy(&buffer[..read]);
        let response = if request.starts_with("GET /status") {
            let status = shared.lock().map(|s| s.clone()).unwrap_or_else(|_| initial_status());
            http_json_response(&serde_json::to_string(&status).unwrap_or_default())
        } else {
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
        };
        let _ = stream.write_all(response.as_bytes());
    }
}

/// Раз в секунду: число игроков и время раунда (раунд = работа сервера).
pub fn update_status(
    time: Res<Time>,
    mut next: Local<f32>,
    players: Res<crate::Players>,
    snapshot: Res<StatusSnapshot>,
) {
    *next -= time.delta_secs();
    if *next > 0.0 {
        return;
    }
    *next = 1.0;
    if let Ok(mut status) = snapshot.0.lock() {
        status.players = players.entries.len() as u32;
        status.round_time_secs = time.elapsed_secs() as u64;
    }
}
