//! Prices from Pricerunner as an external integration for HomewrightHQ.
//!
//! `api` knows the sites and reads the JSON behind Pricerunner's product
//! pages, and this file connects it to HomewrightHQ through the
//! `thehome-plugin` SDK.

pub mod alerts;
pub mod api;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use alerts::{Alerts, Sent};
use api::{Found, LinkError, Prices, Product, Site};
use thehome_plugin::{async_trait, json, Entity, Entry, Error, Flow, FlowReply, Host, Integration, Map, Value};

pub const DOMAIN: &str = "pricerunner";
/// Every other hour: prices in shops do not change by the minute.
const POLL: Duration = Duration::from_secs(2 * 3600);
/// The pause between two products, so a long list does not come as a burst.
const GAP: Duration = Duration::from_secs(2);
/// Failed fetches in a row before a product's entities become unavailable.
const FAILURES: u32 = 3;
/// Offers listed in the attribute `cheapest`.
const CHEAPEST: usize = 5;
/// Products shown from a search.
const FOUND: usize = 10;

/// Logging to HomewrightHQ's log through stderr.
#[doc(hidden)]
pub fn emit(level: &str, message: String) {
    thehome_plugin::log(level, message);
}

#[macro_export]
macro_rules! warn {
    ($($t:tt)*) => { $crate::emit("WARN", format!($($t)*)) };
}

fn str_list(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        Value::String(s) => s.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
        _ => vec![],
    }
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default()
}

fn product_list(v: Option<&Value>) -> Vec<Product> {
    v.and_then(Value::as_array).into_iter().flatten().filter_map(Product::from_json).collect()
}

fn products_json(list: &[Product]) -> Value {
    Value::Array(list.iter().map(Product::to_json).collect())
}

/// Adds the products that are not in the list yet.
fn add(list: &mut Vec<Product>, more: impl IntoIterator<Item = Product>) {
    for p in more {
        if !list.iter().any(|x| x.id == p.id) {
            list.push(p);
        }
    }
}

fn links_field() -> Value {
    json!({
        "type": "string", "title": "Links", "format": "textarea",
        "description": "Links til produkter på Pricerunner, ét pr. linje. Kopiér adressen fra produktets side."
    })
}

fn search_field() -> Value {
    json!({ "type": "string", "title": "Søg", "description": "Eller søg efter et produkt ved navn." })
}

fn user_schema(site: &str) -> Value {
    let options: Vec<Value> = api::SITES.iter().map(|s| json!({ "const": s.key, "title": s.name })).collect();
    json!({
        "type": "object",
        "required": ["site"],
        "properties": {
            "site": { "type": "string", "title": "Land", "oneOf": options, "default": site },
            "links": links_field(),
            "search": search_field(),
        }
    })
}

fn init_schema(current: &[Product], keep: &[String]) -> Value {
    let options: Vec<Value> = current.iter().map(|p| json!({ "const": p.id, "title": p.name })).collect();
    json!({
        "type": "object",
        "properties": {
            "keep": {
                "type": "array", "title": "Produkter", "items": { "type": "string", "oneOf": options }, "default": keep,
                "description": "Fjern fluebenet for at holde op med at følge et produkt."
            },
            "links": links_field(),
            "search": search_field(),
        }
    })
}

fn pick_schema(found: &[Found]) -> Value {
    let options: Vec<Value> = found
        .iter()
        .map(|f| {
            let title = match (f.price, &f.currency) {
                (Some(p), Some(c)) => format!("{} ({} {c})", f.product.name, api::price_state(p)),
                _ => f.product.name.clone(),
            };
            json!({ "const": f.product.id, "title": title })
        })
        .collect();
    json!({
        "type": "object",
        "required": ["found"],
        "properties": {
            "found": { "type": "array", "title": "Fundne produkter", "items": { "type": "string", "oneOf": options }, "default": [] }
        }
    })
}

fn form_with(step: &str, schema: Value, description: Option<&str>, errors: Option<Value>) -> FlowReply {
    let mut r = match errors {
        Some(e) => FlowReply::form_with_errors(step, schema, e),
        None => FlowReply::form(step, schema),
    };
    if let Some(d) = description {
        r.result["description"] = json!(d);
    }
    r
}

/// The values the user typed, kept in the form shown again with an error.
fn keep_input(mut schema: Value, input: &Value) -> Value {
    for key in ["links", "search"] {
        let v = text(input, key);
        if !v.is_empty() {
            schema["properties"][key]["default"] = json!(v);
        }
    }
    schema
}

/// The price alert step, with the site and products kept in the flow.
fn alerts_form(flow: &Flow, site: &Site, list: &[Product], current: &Alerts, errors: Option<Value>, input: Option<&Value>) -> FlowReply {
    let mut schema = alerts::schema(list, current, site.currency);
    if let Some(i) = input {
        schema = alerts::keep_input(schema, i);
    }
    let mut data = flow.data.clone();
    data.insert("site".into(), json!(site.key));
    data.insert("products".into(), products_json(list));
    form_with("alerts", schema, None, errors).with_data(data)
}

/// The alert step: the settings, or the form again.
fn alerts_step(flow: &Flow, site: &Site, current: &Alerts, input: Option<Value>) -> Result<(Vec<Product>, Alerts), Box<FlowReply>> {
    let list = product_list(flow.data.get("products"));
    match input {
        Some(v) => alerts::from_input(&list, &v).map(|a| (list.clone(), a)).map_err(|e| Box::new(alerts_form(flow, site, &list, current, Some(e), Some(&v)))),
        None => Err(Box::new(alerts_form(flow, site, &list, current, None, None))),
    }
}

fn with_alerts(mut data: Value, a: &Alerts) -> Value {
    if let Some(m) = data.as_object_mut() {
        m.extend(a.to_json());
    }
    data
}

fn client() -> Result<reqwest::Client, Error> {
    reqwest::Client::builder()
        .user_agent(concat!("homewright-pricerunner/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(Error::failed)
}

/// Why fetching went wrong: the thing is not there, or the connection failed.
#[derive(Debug)]
enum Fetch {
    Missing,
    Failed(String),
}

impl std::fmt::Display for Fetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fetch::Missing => f.write_str("findes ikke på Pricerunner"),
            Fetch::Failed(e) => f.write_str(e),
        }
    }
}

pub struct Pricerunner {
    /// Another address for every site, for tests.
    base: Option<String>,
    poll: Duration,
    gap: Duration,
    tasks: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

impl Default for Pricerunner {
    fn default() -> Self {
        Self { base: None, poll: POLL, gap: GAP, tasks: Mutex::default() }
    }
}

impl Pricerunner {
    /// With one address for all sites and other intervals, for tests.
    pub fn new(base: &str, poll: Duration, gap: Duration) -> Self {
        Self { base: Some(base.trim_end_matches('/').to_string()), poll, gap, tasks: Mutex::default() }
    }

    fn copy(&self) -> Self {
        Self { base: self.base.clone(), poll: self.poll, gap: self.gap, tasks: Mutex::default() }
    }

    async fn get(&self, client: &reqwest::Client, site: &Site, (path, query): (String, Vec<(&str, String)>)) -> Result<Value, Fetch> {
        let url = format!("{}{path}", self.base.clone().unwrap_or_else(|| site.origin()));
        let r = client.get(url).query(&query).send().await.map_err(|e| Fetch::Failed(e.to_string()))?;
        // the offers of an unknown product are 204 No Content
        if matches!(r.status().as_u16(), 204 | 404) {
            return Err(Fetch::Missing);
        }
        if !r.status().is_success() {
            return Err(Fetch::Failed(format!("HTTP {}", r.status())));
        }
        r.json().await.map_err(|e| Fetch::Failed(e.to_string()))
    }

    async fn product(&self, client: &reqwest::Client, site: &Site, category: &str, id: &str) -> Result<Product, Fetch> {
        let v = self.get(client, site, site.initial_path(category, id)).await?;
        api::parse_initial(&v).ok_or(Fetch::Missing)
    }

    async fn prices(&self, client: &reqwest::Client, site: &Site, p: &Product) -> Result<Prices, Fetch> {
        let v = self.get(client, site, site.offers_path(&p.id)).await?;
        Ok(api::parse_offers(&v, site))
    }

    async fn search(&self, client: &reqwest::Client, site: &Site, q: &str) -> Result<Vec<Found>, Fetch> {
        let v = self.get(client, site, site.search_path(q)).await?;
        Ok(api::parse_search(&v).into_iter().take(FOUND).collect())
    }

    /// The products behind the pasted links, or the error code for the field.
    async fn resolve(&self, site: &'static Site, links: &str) -> Result<Vec<Product>, Value> {
        let mut out = vec![];
        let client = client().map_err(|_| json!({ "base": "cannot_connect" }))?;
        for l in api::split_links(links) {
            let link = match api::parse_link(l) {
                Ok(link) => link,
                Err(LinkError::Bad) => return Err(json!({ "links": "bad_link" })),
                Err(LinkError::OtherSite) => return Err(json!({ "links": "other_site" })),
            };
            if link.site.is_some_and(|s| s != site) {
                return Err(json!({ "links": "other_site" }));
            }
            match self.product(&client, site, &link.category, &link.id).await {
                Ok(p) => add(&mut out, [p]),
                Err(Fetch::Missing) => return Err(json!({ "links": "not_found" })),
                Err(Fetch::Failed(e)) => {
                    warn!("{l}: {e}");
                    return Err(json!({ "base": "cannot_connect" }));
                }
            }
        }
        Ok(out)
    }

    /// The steps shared by the config and the options flow. `current` is set in
    /// the options flow, with the entry's site.
    async fn pick(&self, flow: &Flow, step: &str, input: Option<Value>, current: Option<(&'static Site, &[Product])>) -> Result<Result<(&'static Site, Vec<Product>), FlowReply>, Error> {
        let first = if current.is_some() { "init" } else { "user" };
        let blank = |site: &str, keep: &[String]| match current {
            Some((_, list)) => init_schema(list, keep),
            None => user_schema(site),
        };
        match (step, input) {
            ("pick", Some(v)) => {
                let site = flow.data.get("site").and_then(Value::as_str).and_then(api::site).unwrap_or(&api::SITES[0]);
                let found = product_list(flow.data.get("found"));
                let picked = str_list(&v["found"]);
                let mut list = product_list(flow.data.get("products"));
                add(&mut list, found.iter().filter(|p| picked.contains(&p.id)).cloned());
                if list.is_empty() {
                    let prices = flow.data.get("prices").cloned().unwrap_or_default();
                    let shown: Vec<Found> = found
                        .into_iter()
                        .map(|p| {
                            let price = prices.get(&p.id);
                            Found { price: price.and_then(|x| x[0].as_f64()), currency: price.and_then(|x| x[1].as_str()).map(String::from), product: p }
                        })
                        .collect();
                    return Ok(Err(form_with("pick", pick_schema(&shown), Some("Vælg mindst ét produkt."), Some(json!({ "found": "no_products" })))));
                }
                Ok(Ok((site, list)))
            }
            (_, Some(v)) => {
                let site = match current {
                    Some((s, _)) => s,
                    None => api::site(text(&v, "site")).unwrap_or(&api::SITES[0]),
                };
                // a form sent without the field keeps every product
                let keep_ids = match v.get("keep") {
                    Some(k) if !k.is_null() => str_list(k),
                    _ => current.map(|(_, c)| c.iter().map(|p| p.id.clone()).collect()).unwrap_or_default(),
                };
                let again = |errors: Value| {
                    let schema = keep_input(blank(site.key, &keep_ids), &v);
                    Ok(Err(form_with(first, schema, None, Some(errors))))
                };
                let mut list: Vec<Product> = current.map(|(_, c)| c.iter().filter(|p| keep_ids.contains(&p.id)).cloned().collect()).unwrap_or_default();
                match self.resolve(site, text(&v, "links")).await {
                    Ok(more) => add(&mut list, more),
                    Err(e) => return again(e),
                }
                let q = text(&v, "search");
                if !q.is_empty() {
                    let found = match self.search(&client()?, site, q).await {
                        Ok(f) => f,
                        Err(e) => {
                            warn!("søgning efter {q}: {e}");
                            return again(json!({ "base": "cannot_connect" }));
                        }
                    };
                    if found.is_empty() {
                        return again(json!({ "search": "no_results" }));
                    }
                    let mut data = flow.data.clone();
                    data.insert("site".into(), json!(site.key));
                    data.insert("products".into(), products_json(&list));
                    data.insert("found".into(), Value::Array(found.iter().map(|f| f.product.to_json()).collect()));
                    let prices: Map<String, Value> = found.iter().map(|f| (f.product.id.clone(), json!([f.price, f.currency]))).collect();
                    data.insert("prices".into(), Value::Object(prices));
                    let reply = form_with("pick", pick_schema(&found), Some("Vælg de produkter, du vil følge."), None);
                    return Ok(Err(reply.with_data(data)));
                }
                if list.is_empty() {
                    return again(json!({ "base": "no_products" }));
                }
                Ok(Ok((site, list)))
            }
            (_, None) => {
                let keep: Vec<String> = current.map(|(_, c)| c.iter().map(|p| p.id.clone()).collect()).unwrap_or_default();
                Ok(Err(form_with(first, blank(api::SITES[0].key, &keep), None, None)))
            }
        }
    }
}

fn entry_site(entry: &Entry) -> &'static Site {
    entry.get_str("site").and_then(api::site).unwrap_or(&api::SITES[0])
}

#[async_trait]
impl Integration for Pricerunner {
    async fn config_flow(&self, _host: &Host, flow: Flow, step: &str, input: Option<Value>) -> Result<FlowReply, Error> {
        if step == "alerts" {
            let site = flow.data.get("site").and_then(Value::as_str).and_then(api::site).unwrap_or(&api::SITES[0]);
            return Ok(match alerts_step(&flow, site, &Alerts::default(), input) {
                Err(form) => *form,
                Ok((list, a)) => {
                    let title = format!("Pricerunner {}", site.host.trim_start_matches("www."));
                    FlowReply::create(&title, with_alerts(json!({ "site": site.key, "products": products_json(&list) }), &a))
                }
            });
        }
        match self.pick(&flow, step, input, None).await? {
            Err(form) => Ok(form),
            Ok((site, list)) => Ok(alerts_form(&flow, site, &list, &Alerts::default(), None, None)),
        }
    }

    async fn options_flow(&self, _host: &Host, flow: Flow, step: &str, input: Option<Value>) -> Result<FlowReply, Error> {
        let Some(entry) = flow.entry.clone() else { return Ok(FlowReply::abort("unknown_entry")) };
        let current = product_list(entry.get("products"));
        let site = entry_site(&entry);
        let set = Alerts::from_entry(&entry);
        if step == "alerts" {
            return Ok(match alerts_step(&flow, site, &set, input) {
                Err(form) => *form,
                Ok((list, a)) => FlowReply::create("", with_alerts(json!({ "products": products_json(&list) }), &a)),
            });
        }
        match self.pick(&flow, step, input, Some((site, &current))).await? {
            Err(form) => Ok(form),
            Ok((_, list)) => Ok(alerts_form(&flow, site, &list, &set, None, None)),
        }
    }

    async fn setup_entry(&self, host: &Host, entry: Entry) -> Result<(), Error> {
        let entry_id = entry.entry_id.clone();
        let site = entry_site(&entry);
        let products = product_list(entry.get("products"));
        if products.is_empty() {
            return Err(Error::failed("ingen produkter"));
        }
        let alerts = Alerts::from_entry(&entry).keep(&products);
        let sent = Sent::new(&host.data_dir());
        sent.retain(&entry_id, &alerts);
        let client = client()?;
        // the first fetch here, so a setup without a connection is retried
        let mut first = vec![];
        for (i, p) in products.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(self.gap).await;
            }
            first.push(self.prices(&client, site, p).await);
        }
        if first.iter().all(|r| matches!(r, Err(Fetch::Failed(_)))) {
            if let Some(Err(e)) = first.first() {
                return Err(Error::not_ready(e));
            }
        }

        let mut watched = vec![];
        for product in products {
            let url = site.product_url(&product);
            let device = json!({
                "identifiers": [[DOMAIN, format!("{entry_id}-{}", product.id)]],
                "name": product.name,
                "manufacturer": product.brand.clone().unwrap_or_else(|| "Pricerunner".into()),
                "model": product.category_name.clone().unwrap_or_else(|| "Produkt".into()),
                "entry_type": "service",
                "configuration_url": url,
            });
            let dev = host.request("device/register", json!({ "entry_id": entry_id, "device": device })).await?;
            let dev_id = dev["id"].as_str().ok_or_else(|| Error::failed("intet enheds-id"))?.to_string();
            let price = Entity::new("sensor", format!("{entry_id}-{}-price", product.id))
                .name("Laveste pris")
                .device_class("monetary")
                .unit(site.currency)
                .state_class("measurement")
                .icon("mdi:tag-outline");
            let price = host.add_entity(&entry_id, price.device(&dev_id), false).await?;
            let shop = Entity::new("sensor", format!("{entry_id}-{}-shop", product.id)).name("Billigste butik").icon("mdi:storefront-outline");
            let shop = host.add_entity(&entry_id, shop.device(&dev_id), false).await?;
            watched.push(Watched { product, url, price, shop, failures: 0 });
        }
        for (w, r) in watched.iter_mut().zip(first) {
            match r {
                Ok(p) => {
                    w.show(host, site, &p).await;
                    alerts::check(host, &sent, &alerts, &entry_id, &w.product, &w.url, p.cheapest()).await;
                }
                Err(e) => {
                    warn!("{}: {e}", w.product.name);
                    w.failures = 1;
                }
            }
        }

        let host = host.clone();
        let this = self.copy();
        let id = entry_id.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(this.poll).await;
                for (i, w) in watched.iter_mut().enumerate() {
                    if i > 0 {
                        tokio::time::sleep(this.gap).await;
                    }
                    match this.prices(&client, site, &w.product).await {
                        Ok(p) => {
                            w.failures = 0;
                            w.show(&host, site, &p).await;
                            alerts::check(&host, &sent, &alerts, &id, &w.product, &w.url, p.cheapest()).await;
                        }
                        Err(e) => {
                            w.failures += 1;
                            warn!("{}: {e}", w.product.name);
                            if w.failures == FAILURES {
                                w.unavailable(&host).await;
                            }
                        }
                    }
                }
            }
        });
        if let Some(old) = self.tasks.lock().unwrap().insert(entry_id, task) {
            old.abort();
        }
        Ok(())
    }

    async fn unload_entry(&self, _host: &Host, entry_id: &str) -> Result<(), Error> {
        if let Some(t) = self.tasks.lock().unwrap().remove(entry_id) {
            t.abort();
        }
        Ok(())
    }

    async fn diagnostics(&self, _host: &Host, entry: Entry) -> Value {
        json!({ "data": entry.data, "options": entry.options })
    }
}

/// A product and its two entities.
struct Watched {
    product: Product,
    url: String,
    price: Option<String>,
    shop: Option<String>,
    failures: u32,
}

fn offer_json(o: &api::Offer) -> Value {
    json!({ "shop": o.shop, "price": o.price, "shipping": o.shipping, "stock": o.stock.to_lowercase(), "url": o.url })
}

impl Watched {
    async fn show(&self, host: &Host, site: &Site, prices: &Prices) {
        let cheapest = prices.cheapest();
        let attribution = json!("Priser fra Pricerunner");
        if let Some(id) = &self.price {
            let mut a = Map::new();
            if let Some(o) = cheapest {
                a.insert("shop".into(), json!(o.shop));
                a.insert("shop_url".into(), json!(o.url));
                a.insert("shop_page".into(), json!(o.shop_page));
                a.insert("stock".into(), json!(o.stock.to_lowercase()));
                a.insert("shipping".into(), json!(o.shipping));
                a.insert("price_with_shipping".into(), json!(o.shipping.map(|s| ((o.price + s) * 100.0).round() / 100.0)));
                a.insert("delivery_days".into(), json!(o.delivery_days.map(|(lo, hi)| if lo == hi { lo.to_string() } else { format!("{lo}-{hi}") })));
            }
            a.insert("highest_price".into(), json!(prices.highest));
            a.insert("offers".into(), json!(prices.count));
            let by_shop = prices.by_shop();
            a.insert("shops".into(), json!(by_shop.len()));
            a.insert("shop_prices".into(), Value::Object(by_shop));
            a.insert("cheapest".into(), Value::Array(prices.offers.iter().take(CHEAPEST).map(offer_json).collect()));
            a.insert("product".into(), json!(self.product.name));
            a.insert("product_id".into(), json!(self.product.id));
            a.insert("product_url".into(), json!(self.url));
            if let Some(pic) = &prices.picture {
                a.insert("entity_picture".into(), json!(pic));
            }
            a.insert("currency".into(), json!(cheapest.map(|o| o.currency.as_str()).unwrap_or(site.currency)));
            a.insert("attribution".into(), attribution.clone());
            let state = cheapest.map(|o| api::price_state(o.price)).unwrap_or_else(|| "unknown".into());
            if let Err(e) = host.set_state(id, state, a).await {
                warn!("kunne ikke opdatere {id}: {e}");
            }
        }
        if let Some(id) = &self.shop {
            let mut a = Map::new();
            if let Some(o) = cheapest {
                a.insert("price".into(), json!(o.price));
                a.insert("currency".into(), json!(o.currency));
                a.insert("shop_url".into(), json!(o.url));
                a.insert("shop_page".into(), json!(o.shop_page));
                if let Some(logo) = &o.shop_logo {
                    a.insert("entity_picture".into(), json!(logo));
                }
            }
            a.insert("product_url".into(), json!(self.url));
            a.insert("attribution".into(), attribution);
            let state = cheapest.map(|o| o.shop.clone()).unwrap_or_else(|| "unknown".into());
            if let Err(e) = host.set_state(id, state, a).await {
                warn!("kunne ikke opdatere {id}: {e}");
            }
        }
    }

    async fn unavailable(&self, host: &Host) {
        for id in self.price.iter().chain(&self.shop) {
            let _ = host.set_unavailable(id).await;
        }
    }
}
