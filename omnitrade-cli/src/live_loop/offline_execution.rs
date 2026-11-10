//! Offline tests invoke the production guarded execution function unchanged.
mod replay;
use super::*;
use async_trait::async_trait;
use omnitrade_core::{Order, OrderType};
use omnitrade_exchange::{Account, AccountBalance, ExchangeError, PositionSnapshot};
use std::collections::HashMap;
use std::sync::Mutex;

const SYMBOL: &str = "HYPE-USD_UM_XPERP-310801";
#[derive(Default)]
struct Book {
    qty: f64,
    cash: f64,
    average: f64,
    price: f64,
    fills: usize,
}
#[derive(Clone)]
struct FakeExchange {
    book: std::sync::Arc<Mutex<Book>>,
}
impl FakeExchange {
    fn new() -> Self {
        Self {
            book: std::sync::Arc::new(Mutex::new(Book {
                cash: 50.0,
                price: 100.0,
                ..Book::default()
            })),
        }
    }
}
#[async_trait]
impl ExchangeTrading for FakeExchange {
    async fn place_order(&self, req: &OrderRequest) -> Result<Order, ExchangeError> {
        let mut b = self.book.lock().unwrap();
        let delta = if req.side == Side::Buy {
            req.quantity
        } else {
            -req.quantity
        };
        if b.qty * delta < 0.0 {
            b.cash += b.qty.signum() * delta.abs().min(b.qty.abs()) * (b.price - b.average);
        }
        if b.qty * delta >= 0.0 {
            b.average =
                (b.qty.abs() * b.average + delta.abs() * b.price) / (b.qty.abs() + delta.abs());
        } else if delta.abs() > b.qty.abs() {
            b.average = b.price;
        }
        b.qty += delta;
        b.cash -= req.quantity * b.price * 0.0005;
        b.fills += 1;
        let price = b.price;
        drop(b);
        Ok(Order {
            id: 1,
            symbol: req.symbol.clone(),
            side: req.side,
            order_type: OrderType::Market,
            time_in_force: TimeInForce::GTC,
            price,
            stop_price: 0.0,
            quantity: req.quantity,
            filled_quantity: req.quantity,
            status: OrderStatus::Filled,
            created_at_ms: 0,
            updated_at_ms: 0,
            trailing_highest_price: None,
        })
    }
    async fn cancel_order(&self, _: &str, _: &str) -> Result<(), ExchangeError> {
        Ok(())
    }
    async fn get_open_orders(&self) -> Result<Vec<Order>, ExchangeError> {
        Ok(vec![])
    }
    async fn get_balance(&self) -> Result<Account, ExchangeError> {
        Ok(Account {
            balances: HashMap::from([(
                "USDC".into(),
                AccountBalance {
                    free: self.book.lock().unwrap().cash,
                    locked: 0.0,
                },
            )]),
        })
    }
    async fn position_snapshot(&self, _: &str) -> Result<PositionSnapshot, ExchangeError> {
        let b = self.book.lock().unwrap();
        Ok(PositionSnapshot {
            is_perpetual: true,
            base_qty: b.qty,
            unrealized_pnl: b.qty * (b.price - b.average),
            liq_price: Some(if b.qty >= 0.0 {
                b.price * 0.1
            } else {
                b.price * 1.9
            }),
            mark_price: Some(b.price),
        })
    }
}
async fn step(
    ex: &FakeExchange,
    guard: &mut RiskGuard,
    cap: &mut ExposureLimit,
    side: Side,
) -> Result<Order, LiveLoopError> {
    let price = ex.book.lock().unwrap().price;
    place_order_guarded(
        ex,
        guard,
        &OrderRequest::market(SYMBOL.into(), side, 0.1),
        price,
        50.0,
        Some("USDC"),
        Some(cap),
        None,
    )
    .await
}
#[tokio::test]
async fn offline_cap_rejects_and_opposite_signal_reduces_one_step() {
    let ex = FakeExchange::new();
    let mut guard = RiskGuard::testnet();
    let mut cap = ExposureLimit::new(25.0).allow_shorts(true);
    step(&ex, &mut guard, &mut cap, Side::Buy).await.unwrap();
    step(&ex, &mut guard, &mut cap, Side::Buy).await.unwrap();
    assert!(step(&ex, &mut guard, &mut cap, Side::Buy).await.is_err());
    step(&ex, &mut guard, &mut cap, Side::Sell).await.unwrap();
    let b = ex.book.lock().unwrap();
    assert!((b.qty - 0.1).abs() < 1e-12);
    assert_eq!(b.fills, 3);
}
#[tokio::test]
async fn offline_reduction_still_above_cap_is_rejected() {
    let ex = FakeExchange::new();
    ex.book.lock().unwrap().qty = 0.4;
    let mut guard = RiskGuard::testnet();
    let mut cap = ExposureLimit::new(25.0)
        .allow_shorts(true)
        .adopt_existing(true);
    assert!(step(&ex, &mut guard, &mut cap, Side::Sell).await.is_err());
    assert_eq!(ex.book.lock().unwrap().fills, 0);
}
#[tokio::test]
async fn offline_stateful_fill_accumulates_one_step() {
    let ex = FakeExchange::new();
    let mut guard = RiskGuard::testnet();
    let mut cap = ExposureLimit::new(25.0).allow_shorts(true);
    step(&ex, &mut guard, &mut cap, Side::Buy).await.unwrap();
    assert!((ex.book.lock().unwrap().qty - 0.1).abs() < 1e-12);
}
