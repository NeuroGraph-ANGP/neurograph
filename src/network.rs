use std::net::{TcpListener, TcpStream, SocketAddr};
use std::io::{Write, BufRead, BufReader};
use std::thread;
use std::time::Duration;
use serde::{Serialize, Deserialize};
use serde_json;
use flume;
use std::collections::HashMap;
use crate::transaction::Transaction;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionMessage {
    pub sender: String,
    pub step: u64,
    pub seq: u64,
    pub nonce: u64,
    pub prediction: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationReport {
    pub step: u64,
    pub reputations: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionMessage {
    pub sender: String,
    pub transaction: Transaction,
}

pub fn send_report(step: u64, reputations: &HashMap<String, f64>, observer_addr: &str) {
    let report = ReputationReport { step, reputations: reputations.clone() };
    let addr: SocketAddr = match observer_addr.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Invalid observer address '{}': {}", observer_addr, e);
            return;
        }
    };
    if let Ok(data) = serde_json::to_vec(&report) {
        if let Ok(mut stream) = TcpStream::connect_timeout(
            &addr,
            Duration::from_millis(100)
        ) {
            let _ = stream.write_all(&data);
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
        }
    }
}

pub fn start_tcp_server(
    port: u16,
    tx_pred: flume::Sender<PredictionMessage>,
    tx_tx: flume::Sender<TransactionMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr)?;
    println!("TCP server listening on {}", addr);

    for stream in listener.incoming() {
        let stream = stream?;
        let tx_pred_clone = tx_pred.clone();
        let tx_tx_clone = tx_tx.clone();

        thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();

            while let Ok(bytes_read) = reader.read_line(&mut line) {
                if bytes_read == 0 {
                    break;
                }

                // Trimite linia pentru parsare
                if let Ok(msg) = serde_json::from_str::<PredictionMessage>(&line) {
                    let _ = tx_pred_clone.send(msg);
                } else if let Ok(msg) = serde_json::from_str::<TransactionMessage>(&line) {
                    let _ = tx_tx_clone.send(msg);
                } else {
                    // Ignora liniile goale
                    if !line.trim().is_empty() {
                        eprintln!("Received unknown message type: {}", line);
                    }
                }
                line.clear();
            }
        });
    }
    Ok(())
}

pub fn send_to_peer(addr: SocketAddr, msg: &PredictionMessage) {
    if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
        if let Ok(data) = serde_json::to_vec(msg) {
            let _ = stream.write_all(&data);
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
        }
    }
}

pub fn send_transaction_to_peer(addr: SocketAddr, msg: &TransactionMessage) {
    if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
        if let Ok(data) = serde_json::to_vec(msg) {
            let _ = stream.write_all(&data);
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
        }
    }
}