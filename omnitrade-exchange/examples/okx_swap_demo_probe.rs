//! SWAP (Perpetual) demo verification against OKX EEA demo trading.
//!
//! Credentials from `OKX_API_KEY`, `OKX_API_SECRET`, `OKX_PASSPHRASE` (demo keys),
//! `OKX_REST_BASE=https://eea.okx.com` for EEA demo accounts.
//! Usage: `cargo run -p omnitrade-exchange --example okx_swap_demo_probe -- [SYMBOL] [QTY_BASE]`
//!
//! Verifies the SWAP path end to end on play money: load instrument rules, set leverage,
//! ensure one-way mode, place a small market buy, read the position (pos/upl/liqPx/markPx),
//! then close by selling back the same base quantity.

use omnitrade_core::Side;
use omnitrade_exchange::okx::InstType;
use omnitrade_exchange::{
    ApiCredentials, ExchangeTrading, Network, OkxRestClient, OrderRequest,
};

const DEFAULT_SYMBOL: &str = "BTC-USDT-SWAP";
const DEFAULT_QTY_BASE: f64 = 0.0001; // 0.0001 BTC = 0.01 Kontrakte (ctVal 0.01)

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        eprintln!("{name} ist nicht gesetzt");
        std::process::exit(3);
    })
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let symbol = args.first().map_or(DEFAULT_SYMBOL, String::as_str);
    let qty_base: f64 = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_QTY_BASE);
    // XPERP (instType=FUTURES) vs SWAP: EEA-Konten handeln nur XPERP.
    let inst_type = if symbol.to_uppercase().contains("_XPERP-") {
        InstType::Futures
    } else {
        InstType::Swap
    };

    let creds = ApiCredentials::new(
        env("OKX_API_KEY"),
        env("OKX_API_SECRET"),
        Network::Testnet, // Demo: x-simulated-trading: 1, niemals echtes Geld
    );
    let mut client = OkxRestClient::new(creds, env("OKX_PASSPHRASE")).expect("client");
    if let Ok(base) = std::env::var("OKX_REST_BASE") {
        println!("API-Domain: {base}");
        client = client.with_base_url(base);
    }

    // 1) Instrument-Regeln laden (ctVal/lotSz/minSz).
    let inst = client
        .load_instrument(symbol, inst_type)
        .await
        .expect("load_instrument");
    println!(
        "Instrument: {} ctVal={} lotSz={} minSz={}",
        inst.inst_id, inst.ct_val, inst.lot_sz, inst.min_sz
    );

    // 2) Hebel setzen (isolated, one-way).
    client.set_leverage(symbol, 1).await.expect("set_leverage");
    println!("set_leverage(1) ok");

    // 3) One-way-Modus sicherstellen.
    client.ensure_net_mode().await.expect("ensure_net_mode");
    println!("ensure_net_mode ok (one-way)");

    // 4) Kleine Market-Buy-Order.
    let buy = client
        .place_order(&OrderRequest::market(symbol.into(), Side::Buy, qty_base))
        .await
        .expect("market buy");
    println!("Market-Buy angelegt: id={} (Basis {qty_base})", buy.id);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // 5) Positionsantwort lesen (pos/upl/liqPx/markPx).
    let pos = client
        .position_snapshot(symbol)
        .await
        .expect("position_snapshot");
    println!(
        "Position: pos={} upl={} liqPx={:?} markPx={:?}",
        pos.base_qty, pos.unrealized_pnl, pos.liq_price, pos.mark_price
    );

    // 6) Schließen: gleiche Basis-Menge zurückverkaufen.
    let sell = client
        .place_order(&OrderRequest::market(symbol.into(), Side::Sell, qty_base))
        .await
        .expect("market sell");
    println!("Market-Sell angelegt: id={}", sell.id);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let after = client
        .position_snapshot(symbol)
        .await
        .expect("position_snapshot after");
    println!(
        "Position nach Schließen: pos={} (erwartet 0)",
        after.base_qty
    );
}
