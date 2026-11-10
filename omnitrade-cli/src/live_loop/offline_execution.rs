//! Offline tests invoke the production guarded execution function unchanged.
use super::*;
use async_trait::async_trait;
use omnitrade_core::{Order, OrderType};
use omnitrade_exchange::{Account, AccountBalance, ExchangeError, PositionSnapshot};
use std::collections::HashMap;
use std::sync::Mutex;

const SYMBOL: &str = "HYPE-USD_UM_XPERP-310801";
#[derive(Default)]
struct Book { qty: f64, cash: f64, average: f64, price: f64, fills: usize }
struct FakeExchange { book: Mutex<Book> }
impl FakeExchange {
    fn new() -> Self { Self { book: Mutex::new(Book { cash: 50.0, price: 100.0, ..Book::default() }) } }
}
#[async_trait]
impl ExchangeTrading for FakeExchange {
    async fn place_order(&self, req: &OrderRequest) -> Result<Order, ExchangeError> {
        Ok(Order { id: 1, symbol: req.symbol.clone(), side: req.side,
            order_type: OrderType::Market, time_in_force: TimeInForce::GTC,
            price: self.book.lock().unwrap().price, stop_price: 0.0,
            quantity: req.quantity, filled_quantity: req.quantity,
            status: OrderStatus::Filled, created_at_ms: 0, updated_at_ms: 0,
            trailing_highest_price: None })
    }
    async fn cancel_order(&self, _: &str, _: &str) -> Result<(), ExchangeError> { Ok(()) }
    async fn get_open_orders(&self) -> Result<Vec<Order>, ExchangeError> { Ok(vec![]) }
    async fn get_balance(&self) -> Result<Account, ExchangeError> {
        Ok(Account { balances: HashMap::from([("USDC".into(), AccountBalance { free: self.book.lock().unwrap().cash, locked: 0.0 })]) })
    }
    async fn position_snapshot(&self, _: &str) -> Result<PositionSnapshot, ExchangeError> {
        let b = self.book.lock().unwrap();
        Ok(PositionSnapshot { is_perpetual: true, base_qty: b.qty,
            unrealized_pnl: b.qty * (b.price - b.average),
            liq_price: Some(if b.qty >= 0.0 { b.price * 0.1 } else { b.price * 1.9 }), mark_price: Some(b.price) })
    }
}
async fn step(ex: &FakeExchange, guard: &mut RiskGuard, cap: &mut ExposureLimit, side: Side) -> Result<Order, LiveLoopError> {
    let price = ex.book.lock().unwrap().price;
    place_order_guarded(ex, guard, &OrderRequest::market(SYMBOL.into(), side, 0.1), price, 50.0, Some("USDC"), Some(cap), None).await
}
#[tokio::test]
async fn offline_stateful_fill_accumulates_one_step() {
    let ex = FakeExchange::new();
    let mut guard = RiskGuard::testnet();
    let mut cap = ExposureLimit::new(25.0).allow_shorts(true);
    step(&ex, &mut guard, &mut cap, Side::Buy).await.unwrap();
    assert!((ex.book.lock().unwrap().qty - 0.1).abs() < 1e-12);
}
