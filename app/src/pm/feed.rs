use serde::Deserialize;
use std::{collections::BTreeMap, sync::mpsc::SyncSender, time::Duration};

#[derive(Clone, Deserialize)]
pub struct Level { pub price: String, pub size: String }
impl Level {
    pub fn price(&self) -> Option<f32> {
        self.price.parse::<f32>().ok().filter(|p| (0.0..=1.0).contains(p))
    }
}
#[derive(Clone, Deserialize)]
pub struct Book {
    market: String, asset_id: String, timestamp: String,
    pub bids: Vec<Level>, pub asks: Vec<Level>,
}
impl Book {
    pub fn seconds(&self) -> Option<f64> {
        self.timestamp.parse::<i64>().ok().map(|ms| ms as f64 / 1000.0)
    }
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub window: i64,
    pub books: [Option<Book>; 2],
    pub history: [Vec<(f32, f32, f32)>; 2],
    pub status: String,
}
#[derive(Deserialize)]
struct Event { markets: Vec<Market> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Market {
    slug: String, condition_id: String, outcomes: String, clob_token_ids: String,
    closed: bool, enable_order_book: bool,
}
async fn market(client: &reqwest::Client, window: i64) -> anyhow::Result<(String, [String; 2])> {
    let slug = format!("eth-updown-5m-{window}");
    let event: Event = client.get(format!("https://gamma-api.polymarket.com/events/slug/{slug}"))
        .send().await?.error_for_status()?.json().await?;
    let market = event.markets.into_iter().find(|m| m.slug == slug && !m.closed && m.enable_order_book)
        .ok_or_else(|| anyhow::anyhow!("Active ETH market unavailable"))?;
    let names: Vec<String> = serde_json::from_str(&market.outcomes)?;
    let tokens: Vec<String> = serde_json::from_str(&market.clob_token_ids)?;
    anyhow::ensure!(names.len() == 2 && tokens.len() == 2 && tokens[0] != tokens[1], "Invalid outcome mapping");
    let mut sides: BTreeMap<_, _> = names.into_iter().zip(tokens).collect();
    let up = sides.remove("Up").ok_or_else(|| anyhow::anyhow!("Missing Up token"))?;
    let down = sides.remove("Down").ok_or_else(|| anyhow::anyhow!("Missing Down token"))?;
    Ok((market.condition_id, [up, down]))
}
async fn book(client: &reqwest::Client, condition: &str, token: &str) -> anyhow::Result<Book> {
    let mut b: Book = client.get("https://clob.polymarket.com/book").query(&[("token_id", token)])
        .send().await?.error_for_status()?.json().await?;
    anyhow::ensure!(b.market == condition && b.asset_id == token && b.seconds().is_some(), "Invalid quote identity or time");
    for (levels, bids) in [(&mut b.bids, true), (&mut b.asks, false)] {
        levels.retain(|l| l.price().is_some() && l.size.parse::<f64>().is_ok_and(|q| q.is_finite() && q > 0.));
        levels.sort_by(|a, b| {
            let order = a.price().unwrap().total_cmp(&b.price().unwrap());
            if bids { order.reverse() } else { order }
        });
        levels.truncate(10);
    }
    Ok(b)
}
pub fn spawn(sender: SyncSender<Snapshot>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("async runtime");
        runtime.block_on(async move {
            let client = reqwest::Client::builder().user_agent("PMQuant-Terminal/0.1.1")
                .timeout(Duration::from_secs(8)).build().expect("HTTP client");
            let mut active = None;
            let mut state = Snapshot::default();
            let current = || chrono::Utc::now().timestamp().div_euclid(300) * 300;
            loop {
                let window = current();
                if state.window != window {
                    state = Snapshot { window, status: "Looking up ETH market".into(), ..Default::default() };
                    active = None;
                    if sender.send(state.clone()).is_err() { break; }
                    ctx.request_repaint();
                }
                if active.is_none() {
                    match market(&client, window).await {
                        Ok(found) => active = Some(found),
                        Err(error) => state.status = format!("Market lookup: {error}"),
                    }
                }
                if let Some((condition, tokens)) = &active {
                    let (up, down) = tokio::join!(book(&client, condition, &tokens[0]), book(&client, condition, &tokens[1]));
                    state.status = "Public CLOB snapshots".into();
                    for (index, result) in [up, down].into_iter().enumerate() {
                        match result {
                            Ok(book) => {
                                let offset = (book.seconds().unwrap() - window as f64) as f32;
                                if (0.0..300.0).contains(&offset)
                                    && let (Some(bid), Some(ask)) = (
                                        book.bids.first().and_then(Level::price),
                                        book.asks.first().and_then(Level::price),
                                    )
                                    && state.history[index].last().is_none_or(|last| last.0 < offset)
                                {
                                    state.history[index].push((offset, bid, ask));
                                }
                                if state.books[index].as_ref().and_then(Book::seconds) <= book.seconds() {
                                    state.books[index] = Some(book);
                                }
                            }
                            Err(error) => state.status = format!("Quote update: {error}"),
                        }
                    }
                }
                if current() != window { continue; }
                if sender.send(state.clone()).is_err() { break; }
                ctx.request_repaint();
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    });
}
