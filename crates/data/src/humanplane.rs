//! Read-only adapter for the humanplane backend's public Polymarket endpoints.
use std::time::Duration;

use anyhow::{Result, ensure};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tr_core::{AssetClass, Bar, Timeframe};

use crate::{LiveFeed, MarketData, OrderBook, Quote, SearchHit};

#[derive(Clone)]
pub struct Humanplane {
    client: reqwest::Client,
    api: String,
}

#[derive(Deserialize)]
struct PriceHistory { history: Vec<PricePoint> }
#[derive(Deserialize)]
struct PricePoint { t: i64, p: serde_json::Value }
#[derive(Deserialize)]
struct WireBook { bids: Vec<WireLevel>, asks: Vec<WireLevel> }
#[derive(Deserialize)]
struct WireLevel { price: serde_json::Value, size: serde_json::Value }
#[derive(Deserialize)]
struct SearchResponse { #[serde(default)] events: Vec<Event> }
#[derive(Deserialize)]
struct Event { #[serde(default)] markets: Vec<Market> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Market {
    question: String,
    outcomes: serde_json::Value,
    clob_token_ids: serde_json::Value,
}

pub fn token(symbol: &str) -> Option<&str> {
    let (prefix, id) = symbol.split_once(':')?;
    (prefix.eq_ignore_ascii_case("PM") && (1..=78).contains(&id.len())
        && id.as_bytes().iter().all(u8::is_ascii_digit)).then_some(id)
}

fn strings(value: serde_json::Value) -> Result<Vec<String>> {
    match value {
        serde_json::Value::String(json) => Ok(serde_json::from_str(&json)?),
        other => Ok(serde_json::from_value(other)?),
    }
}

fn number(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok()).filter(|n| n.is_finite())
}

impl Humanplane {
    pub fn new(origin: &str) -> Result<Self> {
        let origin = reqwest::Url::parse(origin)?;
        ensure!(matches!(origin.scheme(), "http" | "https") && origin.host().is_some()
            && origin.username().is_empty() && origin.password().is_none()
            && origin.query().is_none() && origin.fragment().is_none(), "Invalid humanplane origin");
        let client = reqwest::Client::builder().timeout(Duration::from_secs(12)).build()?;
        Ok(Self { client, api: format!("{}/api", origin.as_str().trim_end_matches('/')) })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        Ok(self.client.get(format!("{}/{}", self.api, path)).query(query)
            .send().await?.error_for_status()?.json().await?)
    }
}

#[async_trait]
impl MarketData for Humanplane {
    fn name(&self) -> &str { "humanplane" }

    async fn history(&self, symbol: &str, tf: Timeframe, end: Option<i64>, limit: usize) -> Result<Vec<Bar>> {
        let Some(id) = token(symbol) else { return Ok(Vec::new()) };
        let seconds = tf.seconds().max(60);
        let stop = end.unwrap_or_else(|| chrono::Utc::now().timestamp());
        let span = seconds.saturating_mul(limit.min(4_000) as i64).min(30 * 86_400);
        let query = [
            ("start_ts", stop.saturating_sub(span).to_string()),
            ("end_ts", stop.to_string()),
            ("fidelity", ((seconds / 60).clamp(1, 1_440)).to_string()),
        ];
        let mut rows: PriceHistory = self.get(&format!("history/{id}"), &query).await?;
        rows.history.sort_by_key(|row| row.t);
        let mut bars: Vec<Bar> = Vec::new();
        for row in rows.history {
            let Some(price) = number(&row.p).filter(|p| (0.0..=1.0).contains(p)) else { continue };
            let start = row.t.div_euclid(seconds) * seconds;
            if row.t > stop || end.is_some_and(|end| start >= end) { continue; }
            if let Some(bar) = bars.last_mut().filter(|bar| bar.ts == start) {
                bar.high = bar.high.max(price);
                bar.low = bar.low.min(price);
                bar.close = price;
            } else {
                bars.push(Bar::new(start, price, price, price, price, 0.0));
            }
        }
        Ok(crate::clip(bars, end, limit))
    }

    async fn quotes(&self, symbols: &[String]) -> Result<Vec<Quote>> {
        let mut out = Vec::new();
        for symbol in symbols.iter().filter(|symbol| token(symbol).is_some()) {
            if let Ok(book) = self.order_book(symbol, 1).await
                && let Some(mid) = book.mid()
            {
                out.push(Quote { symbol: symbol.clone(), last: mid, prev_close: mid,
                    ts: chrono::Utc::now().timestamp(), spark: vec![] });
            }
        }
        Ok(out)
    }

    fn live(&self, symbol: &str, tf: Timeframe) -> Option<LiveFeed> {
        token(symbol)?;
        let provider = self.clone();
        let symbol = symbol.to_owned();
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let mut last = None;
            loop {
                if let Ok(bars) = provider.history(&symbol, tf, None, 2).await
                    && let Some(bar) = bars.last().copied()
                {
                    let key = (bar.ts, bar.close, bar.high, bar.low);
                    if last != Some(key) {
                        if sender.send(bar).await.is_err() { return; }
                        last = Some(key);
                    }
                }
                tokio::select! {
                    _ = sender.closed() => return,
                    _ = tokio::time::sleep(Duration::from_secs(8)) => {}
                }
            }
        });
        Some(receiver)
    }

    async fn order_book(&self, symbol: &str, depth: usize) -> Result<OrderBook> {
        let id = token(symbol).ok_or_else(|| anyhow::anyhow!("Select a Polymarket outcome first"))?;
        let book: WireBook = self.get(&format!("book/{id}"), &[]).await?;
        let levels = |rows: Vec<WireLevel>, bids: bool| {
            let mut levels: Vec<_> = rows.into_iter().filter_map(|level| {
                let price = number(&level.price)?;
                let size = number(&level.size)?;
                ((0.0..=1.0).contains(&price) && size > 0.0).then_some((price, size))
            }).collect();
            levels.sort_by(|a, b| if bids { b.0.total_cmp(&a.0) } else { a.0.total_cmp(&b.0) });
            levels.truncate(depth);
            levels
        };
        Ok(OrderBook { symbol: symbol.into(), bids: levels(book.bids, true), asks: levels(book.asks, false),
            source: "Polymarket via humanplane".into(), simulated: false })
    }

    async fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let query = query.trim();
        if query.is_empty() { return Ok(vec![]); }
        let first = query.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
        let markets = if matches!(first.as_str(), "eth" | "btc" | "sol")
            && query.split_whitespace().nth(1).is_some_and(|tf| tf.eq_ignore_ascii_case("5m"))
        {
            let start = chrono::Utc::now().timestamp().div_euclid(300) * 300;
            self.get::<Vec<Market>>("markets", &[("slug", format!("{first}-updown-5m-{start}"))]).await?
        } else {
            let result: SearchResponse = self.get("search", &[("q", query.to_owned()),
                ("limit_per_type", "12".to_owned())]).await?;
            result.events.into_iter().flat_map(|e| e.markets).collect()
        };
        let mut hits = Vec::new();
        for market in markets {
            let (Ok(names), Ok(tokens)) = (strings(market.outcomes), strings(market.clob_token_ids)) else { continue };
            if names.len() != tokens.len() { continue; }
            for (name, id) in names.into_iter().zip(tokens) {
                let key = format!("PM:{id}");
                if token(&key).is_some() {
                    hits.push(SearchHit { key, name: format!("{} · {}", market.question, name),
                        class: AssetClass::Crypto, exchange: Some("Polymarket".into()) });
                }
            }
        }
        Ok(hits)
    }
}
