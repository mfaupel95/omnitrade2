//! Drives the real loop on a single-thread runtime; no network adapter exists here.
use super::*;
use omnitrade_exchange::DepthUpdate;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
struct Replay {
    candles: Vec<Candle>,
    exchange: FakeExchange,
    status: PathBuf,
    rows: Arc<Mutex<Vec<Value>>>,
}
#[async_trait]
impl ExchangeStream for Replay {
    async fn connect(&mut self) -> Result<(), ExchangeError> {
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<(), ExchangeError> {
        Ok(())
    }
    async fn subscribe_depth(
        &mut self,
        _: &str,
    ) -> Result<mpsc::Receiver<DepthUpdate>, ExchangeError> {
        Err(ExchangeError::ChannelClosed)
    }
    async fn subscribe_candles(
        &mut self,
        _: &str,
        _: &str,
    ) -> Result<mpsc::Receiver<Candle>, ExchangeError> {
        let (tx, rx) = mpsc::channel(1);
        let candles = std::mem::take(&mut self.candles);
        let ex = self.exchange.clone();
        let status = self.status.clone();
        let rows = self.rows.clone();
        tokio::spawn(async move {
            for candle in candles {
                ex.book.lock().expect("book").price = candle.close;
                tx.send(candle.clone()).await.expect("live receiver");
                // All fake trading awaits are immediately ready. The real loop runs
                // through its complete candle before blocking on the empty receiver.
                tokio::task::yield_now().await;
                let s: Value =
                    serde_json::from_slice(&fs::read(&status).expect("status")).expect("json");
                assert_eq!(s["last_candle_ms"].as_u64(), Some(candle.timestamp_ms));
                let b = ex.book.lock().expect("book");
                rows.lock().expect("rows").push(json!({"timestamp_ms":candle.timestamp_ms,"signal":s["last_signal"],"position":b.qty,"fills":b.fills,"accepted":s["orders_placed"],"rejected":s["orders_rejected"],"price":b.price,"quantity":0.1,"equity":b.cash+b.qty*(b.price-b.average),"last_order":s["recent_orders"].as_array().and_then(|a|a.last())}));
            }
        });
        Ok(rx)
    }
}
#[tokio::test]
async fn offline_hype_real_loop() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let fixture = root.join("offline-artifacts/fixture");
    let mut reader = csv::Reader::from_path(fixture.join("candles.csv")).expect("fixture");
    let candles = reader
        .records()
        .map(|r| {
            let r = r.expect("csv");
            Candle::new(
                r[0].parse().expect("timestamp"),
                r[1].parse().expect("open"),
                r[2].parse().expect("high"),
                r[3].parse().expect("low"),
                r[4].parse().expect("close"),
                r[5].parse().expect("volume"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(candles.len(), 1440);
    let path = std::env::temp_dir().join(format!("offline-hype-{}.json", std::process::id()));
    let ex = FakeExchange::new();
    let rows = Arc::new(Mutex::new(Vec::new()));
    let stream = Replay {
        candles,
        exchange: ex.clone(),
        status: path.clone(),
        rows: rows.clone(),
    };
    let script = fixture
        .join("momentum_ema_rsi.omni")
        .to_string_lossy()
        .into_owned();
    let kill = path.with_extension("kill");
    assert!(!kill.exists());
    let config = LiveConfig {
        script_path: &script,
        symbol: SYMBOL,
        network: Network::Testnet,
        initial_capital: 50.0,
        order_quantity: 0.1,
        kill_file: &kill,
        max_notional: Some(25.0),
        interval: "1m",
        allow_shorts: true,
        warmup: &[],
    };
    let mut status =
        StatusWriter::new(Some(path.clone()), "fake", SYMBOL, "testnet", &script, 50.0);
    live_loop(
        &config,
        Some("USDC"),
        stream,
        ex,
        &mut RiskGuard::testnet(),
        &mut status,
    )
    .await
    .expect("real loop");
    let rows = rows.lock().expect("rows");
    assert_eq!(rows.len(), 1440);
    fs::write(
        root.join("offline-artifacts/hype-replay.json"),
        serde_json::to_vec_pretty(&*rows).expect("serialize"),
    )
    .expect("ledger");
    assert_eq!(status.status().orders_placed, 107);
    assert_eq!(status.status().orders_rejected, 99);
    assert!((status.status().net_quantity - 0.3).abs() < 1e-10);
    fs::remove_file(path).expect("remove scratch status");
}
