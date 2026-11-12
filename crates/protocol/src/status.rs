//! Статус сервера для лаунчера — как `GET /status` у сервера SS14
//! (`Robust.Server/ServerStatus/StatusHost.cs`, `StatusHost.Handlers.cs`):
//! JSON с названием, числом игроков, мягким лимитом и временем раунда. Лаунчер
//! SS14 показывает их строкой списка серверов («1ч 42м», «46 / 100»).
//!
//! У нас HTTP слушает тот же номер порта, что и игра, но по TCP (игра — UDP).

use serde::{Deserialize, Serialize};

/// Ответ `/status` (поля — подмножество ответа SS14, имена те же).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub name: String,
    pub players: u32,
    pub soft_max_players: u32,
    /// Секунд с начала раунда (в SS14 — `round_start_time`, ISO-время).
    pub round_time_secs: u64,
    /// Версия протокола: лаунчер не пускает на несовместимый сервер.
    pub protocol_version: u32,
}

/// Время раунда как в лаунчере SS14: «1ч 42м», «26м», «0м».
pub fn format_round_time(secs: u64) -> String {
    let minutes = secs / 60;
    let (hours, minutes) = (minutes / 60, minutes % 60);
    if hours > 0 {
        format!("{hours}ч {minutes}м")
    } else {
        format!("{minutes}м")
    }
}

/// HTTP-запрос статуса (минимальный, как шлёт лаунчер).
pub fn status_request(host: &str) -> String {
    format!("GET /status HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
}

/// HTTP-ответ с телом JSON.
pub fn http_json_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\n\
         Access-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Тело HTTP-ответа (после пустой строки), `None` — не ответ 200.
pub fn http_body(response: &str) -> Option<&str> {
    let (head, body) = response.split_once("\r\n\r\n")?;
    head.starts_with("HTTP/1.1 200").then_some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_time_matches_launcher() {
        assert_eq!(format_round_time(0), "0м");
        assert_eq!(format_round_time(26 * 60 + 59), "26м");
        assert_eq!(format_round_time(102 * 60), "1ч 42м");
    }

    #[test]
    fn http_roundtrip() {
        let response = http_json_response("{\"a\":1}");
        assert_eq!(http_body(&response), Some("{\"a\":1}"));
        assert_eq!(http_body("HTTP/1.1 404 Not Found\r\n\r\nx"), None);
        assert!(status_request("127.0.0.1:7777").starts_with("GET /status HTTP/1.1\r\n"));
    }
}
