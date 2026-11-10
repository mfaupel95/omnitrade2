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
    fees: f64,
    events: Vec<serde_json::Value>,
    next_fill: Option<Result<(f64, f64), ExchangeError>>,
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
        use serde_json::json;
        let id = b.events.len() / 2 + 1;
        let seq = b.events.len() + 1;
        b.events.push(json!({"seq":seq,"kind":"request","id":id,"symbol":req.symbol,"side":format!("{:?}",req.side),"order_type":format!("{:?}",req.order_type),"quantity":req.quantity,"raw_price":req.price}));
        let result = if req.order_type != OrderType::Market
            || req.symbol != SYMBOL
            || !req.quantity.is_finite()
            || req.quantity <= 0.0
            || req.price.is_some()
        {
            Err(ExchangeError::InvalidRequest(
                "unsupported fake request".into(),
            ))
        } else {
            let price = b.price;
            b.next_fill.take().unwrap_or(Ok((price, req.quantity)))
        };
        let (price, filled) = match result {
            Ok((p, q))
                if p.is_finite() && p > 0.0 && q.is_finite() && q > 0.0 && q <= req.quantity =>
            {
                (p, q)
            }
            other => {
                let error = other
                    .err()
                    .unwrap_or(ExchangeError::InvalidRequest("invalid fake fill".into()));
                let seq = b.events.len() + 1;
                b.events
                    .push(json!({"seq":seq,"kind":"error","id":id,"error":error.to_string()}));
                return Err(error);
            }
        };
        let delta = if req.side == Side::Buy {
            filled
        } else {
            -filled
        };
        if b.qty * delta < 0.0 {
            b.cash += b.qty.signum() * delta.abs().min(b.qty.abs()) * (price - b.average);
        }
        if b.qty * delta >= 0.0 {
            b.average =
                (b.qty.abs() * b.average + delta.abs() * price) / (b.qty.abs() + delta.abs());
        } else if delta.abs() > b.qty.abs() {
            b.average = price;
        }
        b.qty += delta;
        if b.qty.abs() < 1e-12 {
            b.qty = 0.0;
            b.average = 0.0;
        }
        let fee = filled * price * 0.0005;
        b.cash -= fee;
        b.fees += fee;
        b.fills += 1;
        let status = if filled < req.quantity {
            OrderStatus::PartiallyFilled
        } else {
            OrderStatus::Filled
        };
        let seq = b.events.len() + 1;
        b.events.push(json!({"seq":seq,"kind":"response","id":id,"symbol":req.symbol,"side":format!("{:?}",req.side),"order_type":"Market","quantity":req.quantity,"price":price,"filled_quantity":filled,"status":format!("{:?}",status),"fee":fee}));
        drop(b);
        Ok(Order {
            id: id as u64,
            symbol: req.symbol.clone(),
            side: req.side,
            order_type: OrderType::Market,
            time_in_force: TimeInForce::GTC,
            price,
            stop_price: 0.0,
            quantity: req.quantity,
            filled_quantity: filled,
            status,
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

#[tokio::test]
async fn offline_unknown_order_type_fails_closed() {
    let ex = FakeExchange::new();
    let req = OrderRequest::limit(SYMBOL.into(), Side::Buy, 0.1, 100.0);
    assert!(ex.place_order(&req).await.is_err());
    assert_eq!(ex.book.lock().unwrap().fills, 0);
}

#[tokio::test]
async fn offline_actual_partial_fill_and_error_evidence() {
    let ex = FakeExchange::new();
    ex.book.lock().unwrap().next_fill = Some(Ok((101.0, 0.04)));
    let req = OrderRequest::market(SYMBOL.into(), Side::Buy, 0.1);
    let order = ex.place_order(&req).await.unwrap();
    assert_eq!(order.price, 101.0);
    assert_eq!(order.filled_quantity, 0.04);
    assert_eq!(order.status, OrderStatus::PartiallyFilled);
    assert_eq!(ex.book.lock().unwrap().events[1]["filled_quantity"], 0.04);
    ex.book.lock().unwrap().next_fill = Some(Err(ExchangeError::ApiError("injected".into())));
    assert!(ex.place_order(&req).await.is_err());
    let b = ex.book.lock().unwrap();
    assert_eq!(b.events.len(), 4);
    assert_eq!(b.events[3]["kind"], "error");
    assert_eq!(b.qty, 0.04);
}

#[tokio::test]
async fn offline_short_profit_loss_and_cross_zero_book() {
    for (cover_price, quantity, cash, position, average) in [
        (90.0, 0.1, 50.9855, -0.1, 100.0),
        (110.0, 0.1, 48.9845, -0.1, 100.0),
        (90.0, 0.3, 51.9765, 0.1, 90.0),
        (90.0, 0.2, 51.981, 0.0, 0.0),
    ] {
        let ex = FakeExchange::new();
        let first = ex
            .place_order(&OrderRequest::market(SYMBOL.into(), Side::Sell, 0.2))
            .await
            .unwrap();
        ex.book.lock().unwrap().price = cover_price;
        let second = ex
            .place_order(&OrderRequest::market(SYMBOL.into(), Side::Buy, quantity))
            .await
            .unwrap();
        assert_ne!(first.id, second.id);
        let b = ex.book.lock().unwrap();
        assert!((b.cash - cash).abs() < 1e-10);
        assert!((b.qty - position).abs() < 1e-10);
        assert!((b.average - average).abs() < 1e-10);
    }
}
