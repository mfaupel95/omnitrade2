//! SWAP-spezifische Teile des OKX-REST-Clients: Instrumentregeln laden, Hebel setzen,
//! Positionsmodus prüfen und Mengen zwischen Basiswährung und Kontrakten umrechnen.

use std::collections::HashMap;
use std::sync::RwLock;

use omnitrade_core::Order;
use reqwest::Method;
use serde_json::Value;

use crate::error::ExchangeError;
use crate::okx::instrument::{InstType, Instrument};
use crate::okx::rest::OkxRestClient;
use crate::okx::swap;
use crate::trading::{OrderRequest, PositionSnapshot};

/// Wertet die Antwort von `/account/positions` für ein SWAP-Instrument aus.
///
/// Fehlt ein Feld, ist das ein Fehler: ein stilles `0` würde das Risiko-Limit
/// unbemerkt außer Kraft setzen. Ein leerer Text (OKX für „nicht zutreffend“) zählt als 0.
pub(crate) fn parse_swap_position(
    data: &[Value],
    inst: &Instrument,
) -> Result<PositionSnapshot, ExchangeError> {
    let mut snapshot = PositionSnapshot {
        is_perpetual: true,
        ..PositionSnapshot::default()
    };
    for item in data {
        if item.get("instId").and_then(Value::as_str) != Some(inst.inst_id.as_str()) {
            continue;
        }
        let contracts = optional_number(item, "pos")?;
        snapshot.base_qty += inst.contracts_to_base(contracts);
        snapshot.unrealized_pnl += optional_number(item, "upl")?;
        if contracts != 0.0 && snapshot.liq_price.is_none() {
            snapshot.liq_price = optional_price(item, "liqPx")?;
            snapshot.mark_price = optional_price(item, "markPx")?;
        }
    }
    Ok(snapshot)
}

/// Preisfeld: `None` bei leerem Text, Fehler bei fehlendem oder ungültigem Feld.
fn optional_price(item: &Value, key: &str) -> Result<Option<f64>, ExchangeError> {
    let value = optional_number(item, key)?;
    Ok((value > 0.0).then_some(value))
}

fn optional_number(item: &Value, key: &str) -> Result<f64, ExchangeError> {
    let raw = item
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ExchangeError::AccountError(format!("Position ohne Feld '{key}'")))?;
    if raw.is_empty() {
        return Ok(0.0);
    }
    raw.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| ExchangeError::AccountError(format!("Feld '{key}' ist keine Zahl: '{raw}'")))
}

/// Erkennt kontraktbasierte Instrumente an der OKX-Notation:
/// SWAP (`BTC-USDT-SWAP`), XPERP (`BTC-USD_UM_XPERP-310404`) und datierte
/// Futures (`BTC-USD_UM-261030`). Die Regel steht EINMAL in
/// [`crate::okx::instrument::is_contract_id`].
#[must_use]
pub(crate) fn is_contract_id(inst_id: &str) -> bool {
    crate::okx::instrument::is_contract_id(inst_id)
}

/// Cache der Instrumentregeln, befüllt durch [`OkxRestClient::load_instrument`].
pub(crate) type InstrumentCache = RwLock<HashMap<String, Instrument>>;

fn cache_poisoned() -> ExchangeError {
    ExchangeError::ApiError("Instrument-Cache nicht verfügbar".to_string())
}

impl OkxRestClient {
    /// Lädt die Handelsregeln eines Instruments und merkt sie für die Umrechnung vor.
    ///
    /// # Errors
    /// `ApiError`, wenn OKX das Instrument nicht kennt oder die Regeln ungültig sind.
    pub async fn load_instrument(
        &self,
        inst_id: &str,
        inst_type: InstType,
    ) -> Result<Instrument, ExchangeError> {
        let path = format!(
            "/api/v5/public/instruments?instType={}&instId={}",
            inst_type.as_api_str(),
            urlencoding::encode(inst_id)
        );
        let data = self.call(Method::GET, &path, None).await?;
        let inst = data
            .first()
            .ok_or_else(|| ExchangeError::ApiError(format!("{inst_id}: Instrument unbekannt")))
            .and_then(Instrument::parse)?;
        self.instruments
            .write()
            .map_err(|_| cache_poisoned())?
            .insert(inst.inst_id.clone(), inst.clone());
        Ok(inst)
    }

    /// Setzt den Hebel (isolated) für ein Instrument. Deckel: [`swap::MAX_LEVERAGE`].
    ///
    /// # Errors
    /// `InvalidRequest` bei Hebel außerhalb des Bereichs, sonst API-Fehler.
    pub async fn set_leverage(&self, inst_id: &str, lever: u32) -> Result<(), ExchangeError> {
        let body = swap::leverage_body(inst_id, lever)?;
        self.call(Method::POST, "/api/v5/account/set-leverage", Some(&body))
            .await
            .map(|_| ())
    }

    /// Stellt sicher, dass das Konto im one-way-Modus läuft. Fail closed.
    ///
    /// # Errors
    /// `AccountError` im Hedge-Modus, sonst API-Fehler.
    pub async fn ensure_net_mode(&self) -> Result<(), ExchangeError> {
        let data = self
            .call(Method::GET, "/api/v5/account/config", None)
            .await?;
        swap::check_position_mode(&data)
    }

    /// Liefert die gecachte SWAP-Regel oder einen Fehler mit Handlungsanweisung.
    pub(crate) fn swap_instrument(&self, inst_id: &str) -> Result<Instrument, ExchangeError> {
        let cached = self
            .instruments
            .read()
            .map_err(|_| cache_poisoned())?
            .get(inst_id)
            .cloned();
        cached
            .filter(|i| i.inst_type.is_contract_based())
            .ok_or_else(|| {
                ExchangeError::InvalidRequest(format!(
                    "{inst_id}: SWAP/XPERP-Instrument nicht geladen (load_instrument zuerst)"
                ))
            })
    }

    /// Baut den Order-Body für SWAP/XPERP. Die Menge wird auf Kontrakte gerundet, bevor sie
    /// den Adapter verlässt. Für SWAP ist quantity Base-Währung; für XPERP (Futures) Kontrakte.
    pub(crate) fn swap_order_body(
        &self,
        req: &OrderRequest,
        px: Option<f64>,
    ) -> Result<Value, ExchangeError> {
        let inst = self.swap_instrument(&req.symbol)?;
        let contracts = if inst.inst_type.is_contract_based() && inst.inst_type != InstType::Swap {
            // XPERP (Futures): quantity is already in contracts (ctVal in settlement currency)
            req.quantity
        } else {
            // SWAP: quantity is in base currency, convert using ctVal
            inst.base_to_contracts(req.quantity)?
        };
        let text = swap::format_contracts(contracts, inst.lot_sz);
        Ok(swap::order_body(req, &text, px))
    }

    /// Liest die Position eines SWAP-Instruments (`/account/positions`).
    pub(crate) async fn fetch_swap_position(
        &self,
        inst_id: &str,
    ) -> Result<PositionSnapshot, ExchangeError> {
        let inst = self.swap_instrument(inst_id)?;
        let path = format!(
            "/api/v5/account/positions?instType={}&instId={}",
            inst.inst_type.as_api_str(),
            urlencoding::encode(inst_id)
        );
        let data = self.call(Method::GET, &path, None).await?;
        parse_swap_position(&data, &inst)
    }

    /// Rechnet Kontraktmengen einer SWAP-Order in Basiswährung zurück. SPOT bleibt unberührt.
    pub(crate) fn to_base_units(&self, mut order: Order) -> Result<Order, ExchangeError> {
        if !is_contract_id(&order.symbol) {
            return Ok(order);
        }
        let inst = self.swap_instrument(&order.symbol)?;
        order.quantity = inst.contracts_to_base(order.quantity);
        order.filled_quantity = inst.contracts_to_base(order.filled_quantity);
        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn btc_swap() -> Instrument {
        Instrument::parse(&json!({
            "instId": "BTC-USDT-SWAP", "instType": "SWAP", "ctVal": "0.01",
            "lotSz": "0.01", "minSz": "0.01",
        }))
        .expect("fixture parses")
    }

    #[test]
    fn long_position_converts_contracts_and_carries_upl() {
        // Arrange: 5 Kontrakte long, Verlust -12.5 USDT
        let data = [
            json!({"instId": "BTC-USDT-SWAP", "pos": "5", "upl": "-12.5", "liqPx": "", "markPx": ""}),
        ];
        // Act
        let snap = parse_swap_position(&data, &btc_swap()).expect("parses");
        // Assert
        assert!(snap.is_perpetual);
        assert!((snap.base_qty - 0.05).abs() < 1e-12);
        assert!((snap.unrealized_pnl + 12.5).abs() < 1e-12);
    }

    #[test]
    fn short_position_is_negative_in_base() {
        let data = [
            json!({"instId": "BTC-USDT-SWAP", "pos": "-3", "upl": "4", "liqPx": "", "markPx": ""}),
        ];
        let snap = parse_swap_position(&data, &btc_swap()).expect("parses");
        assert!((snap.base_qty + 0.03).abs() < 1e-12);
    }

    #[test]
    fn empty_result_means_flat_perpetual() {
        let snap = parse_swap_position(&[], &btc_swap()).expect("parses");
        assert!(snap.is_perpetual);
        assert_eq!(snap.base_qty, 0.0);
        assert_eq!(snap.unrealized_pnl, 0.0);
    }

    #[test]
    fn empty_text_counts_as_zero_but_missing_field_fails_closed() {
        let flat = [json!({"instId": "BTC-USDT-SWAP", "pos": "0", "upl": ""})];
        assert_eq!(
            parse_swap_position(&flat, &btc_swap())
                .expect("empty is zero")
                .unrealized_pnl,
            0.0
        );
        let missing = [json!({"instId": "BTC-USDT-SWAP", "pos": "1"})];
        assert!(parse_swap_position(&missing, &btc_swap()).is_err());
    }

    #[test]
    fn open_position_carries_mark_and_liquidation_price() {
        let data = [json!({
            "instId": "BTC-USDT-SWAP", "pos": "2", "upl": "1.0",
            "liqPx": "45000.5", "markPx": "50000"
        })];
        let snap = parse_swap_position(&data, &btc_swap()).expect("parses");
        assert_eq!(snap.liq_price, Some(45_000.5));
        assert_eq!(snap.mark_price, Some(50_000.0));
    }

    #[test]
    fn flat_position_has_no_liquidation_price() {
        let data = [json!({
            "instId": "BTC-USDT-SWAP", "pos": "0", "upl": "", "liqPx": "", "markPx": ""
        })];
        let snap = parse_swap_position(&data, &btc_swap()).expect("parses");
        assert_eq!(snap.liq_price, None);
        assert_eq!(snap.mark_price, None);
    }

    #[test]
    fn contract_id_recognizes_swap_and_xperp() {
        assert!(is_contract_id("BTC-USDT-SWAP"));
        assert!(is_contract_id("BTC-USD_UM_XPERP-310404"));
        assert!(is_contract_id("btc-usd_um_xperp-310404"));
        assert!(!is_contract_id("BTC-USDT"));
        assert!(!is_contract_id("BTC-USDC"));
    }

    /// Echter Demo-End-to-End-Test: XPERP-Instrument laden (braucht Env-Variablen
    /// OKX_DEMO_API_KEY/SECRET/PASSPHRASE und OKX_REST_BASE=https://eea.okx.com).
    /// Wird nur ausgeführt, wenn alle Env-Variablen gesetzt sind.
    #[test]
    #[ignore]
    fn demo_xperp_load_instrument() {
        use crate::{ApiCredentials, Network, OkxRestClient};
        let key = std::env::var("OKX_DEMO_API_KEY").ok();
        let secret = std::env::var("OKX_DEMO_API_SECRET").ok();
        let pass = std::env::var("OKX_DEMO_API_PASSPHRASE").ok();
        let base = std::env::var("OKX_REST_BASE").ok();
        let (Some(key), Some(secret), Some(pass), Some(base)) = (key, secret, pass, base) else {
            eprintln!("Demo-Env nicht gesetzt, Test übersprungen");
            return;
        };
        eprintln!(
            "DEBUG_TEST: key={}... secret={}... pass={}... base={}",
            &key[..8.min(key.len())],
            &secret[..8.min(secret.len())],
            &pass[..8.min(pass.len())],
            base
        );
        let creds = ApiCredentials::new(key, secret, Network::Testnet);
        let mut client = OkxRestClient::new(creds, pass).expect("client");
        client = client.with_base_url(base);
        eprintln!("DEBUG_TEST: calling load_instrument with BTC-USD-261030, Futures");
        let inst =
            tokio_test::block_on(client.load_instrument("BTC-USD-261030", InstType::Futures))
                .expect("load_instrument");
        assert_eq!(inst.inst_type, InstType::Futures);
        assert!(inst.inst_type.is_contract_based());
        eprintln!("ct_val={}", inst.ct_val);
        assert!(inst.ct_val > 0.0);
        eprintln!("OK: geladen {} ctVal={}", inst.inst_id, inst.ct_val);
    }

    #[test]
    fn xperp_position_parses_like_swap() {
        // Arrange: 3 Kontrakte long, ctVal 0.0001 -> 0.0003 BTC
        let inst = Instrument::parse(&json!({
            "instId": "BTC-USD_UM_XPERP-310404", "instType": "FUTURES", "ctVal": "0.0001",
            "lotSz": "1", "minSz": "1",
        }))
        .expect("fixture parses");
        let data = [json!({
            "instId": "BTC-USD_UM_XPERP-310404", "pos": "3", "upl": "1.5",
            "liqPx": "45000.5", "markPx": "50000"
        })];
        // Act
        let snap = parse_swap_position(&data, &inst).expect("parses");
        // Assert
        assert!(snap.is_perpetual);
        assert!((snap.base_qty - 0.0003).abs() < 1e-12);
        assert!((snap.unrealized_pnl - 1.5).abs() < 1e-12);
        assert_eq!(snap.liq_price, Some(45_000.5));
        assert_eq!(snap.mark_price, Some(50_000.0));
    }

    #[test]
    fn other_instruments_are_ignored() {
        let data = [json!({"instId": "ETH-USDT-SWAP", "pos": "9", "upl": "99"})];
        let snap = parse_swap_position(&data, &btc_swap()).expect("parses");
        assert_eq!(snap.base_qty, 0.0);
    }
}
