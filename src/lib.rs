//! Prices from Pricerunner as an external integration for HomewrightHQ.
//!
//! `api` knows the sites and reads the JSON behind Pricerunner's product
//! pages, and this file connects it to HomewrightHQ through the
//! `thehome-plugin` SDK.
//!
//! An entry follows its products in one or more countries. The first country
//! it was set up with is kept in `site`: its entities and alerts have no
//! country in their ids, the way they had before there could be several.

pub mod alerts;
pub mod api;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use alerts::{Alerts, Sent, Watch};
use api::{Found, LinkError, Prices, Product, Site};
use thehome_plugin::{async_trait, json, Entity, Entry, Error, Flow, FlowReply, Host, Integration, Map, Value};

pub const DOMAIN: &str = "pricerunner";
/// Every other hour: prices in shops do not change by the minute.
const POLL: Duration = Duration::from_secs(2 * 3600);
/// The pause between two fetches, so a long list does not come as a burst.
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

/// The known sites among keys, in the order of `api::SITES`.
fn sites_of(v: &Value) -> Vec<&'static Site> {
    let keys = str_list(v);
    api::SITES.iter().filter(|s| keys.iter().any(|k| k == s.key)).collect()
}

/// The countries an entry checks.
#[derive(Clone, Debug)]
struct Choice {
    sites: Vec<&'static Site>,
    /// The entry's first country, whose ids have no country in them.
    home: &'static Site,
    /// Shops abroad that sell to the country count too.
    foreign: bool,
}

impl Default for Choice {
    fn default() -> Self {
        Self { sites: vec![&api::SITES[0]], home: &api::SITES[0], foreign: false }
    }
}

impl Choice {
    /// Where to search and look up a bare id.
    fn main(&self) -> &'static Site {
        if self.sites.contains(&self.home) {
            self.home
        } else {
            self.sites.first().copied().unwrap_or(self.home)
        }
    }

    fn keys(&self) -> Value {
        json!(self.sites.iter().map(|s| s.key).collect::<Vec<_>>())
    }

    /// Kept in the flow between the steps.
    fn insert(&self, data: &mut Map<String, Value>) {
        data.insert("sites".into(), self.keys());
        data.insert("home".into(), json!(self.home.key));
        data.insert("foreign".into(), json!(self.foreign));
    }

    fn from_flow(data: &Map<String, Value>) -> Self {
        let sites = data.get("sites").map(sites_of).filter(|s| !s.is_empty()).unwrap_or_else(|| vec![&api::SITES[0]]);
        let home = data.get("home").and_then(Value::as_str).and_then(api::site).unwrap_or(sites[0]);
        Self { sites, home, foreign: data.get("foreign").and_then(Value::as_bool).unwrap_or(false) }
    }

    fn from_entry(entry: &Entry) -> Self {
        let home = entry.get_str("site").and_then(api::site).unwrap_or(&api::SITES[0]);
        let sites = entry.get("sites").map(sites_of).filter(|s| !s.is_empty()).unwrap_or_else(|| vec![home]);
        Self { sites, home, foreign: entry.get("foreign").and_then(Value::as_bool).unwrap_or(false) }
    }
}

fn links_field() -> Value {
    json!({
        "type": "string", "title": "Links", "format": "textarea",
        "description": "Links til produkter på Pricerunner eller Klarna, ét pr. linje. Kopiér adressen fra produktets side."
    })
}

fn search_field() -> Value {
    json!({ "type": "string", "title": "Søg", "description": "Eller søg efter et produkt ved navn." })
}

fn sites_field(c: &Choice) -> Value {
    let options: Vec<Value> = api::SITES.iter().map(|s| json!({ "const": s.key, "title": s.name })).collect();
    json!({
        "type": "array", "title": "Lande", "items": { "type": "string", "oneOf": options }, "default": c.keys(),
        "description": "Hvert land får sin egen pris i landets valuta."
    })
}

fn foreign_field(c: &Choice) -> Value {
    json!({
        "type": "boolean", "title": "Medtag udenlandske butikker", "default": c.foreign,
        "description": "Butikker i andre lande, der sælger til landet, fx Amazon.de på pricerunner.dk."
    })
}

fn user_schema(c: &Choice) -> Value {
    json!({
        "type": "object",
        "required": ["sites"],
        "properties": {
            "sites": sites_field(c),
            "foreign": foreign_field(c),
            "links": links_field(),
            "search": search_field(),
        }
    })
}

fn init_schema(current: &[Product], keep: &[String], c: &Choice) -> Value {
    let options: Vec<Value> = current.iter().map(|p| json!({ "const": p.id, "title": p.name })).collect();
    json!({
        "type": "object",
        "properties": {
            "keep": {
                "type": "array", "title": "Produkter", "items": { "type": "string", "oneOf": options }, "default": keep,
                "description": "Fjern fluebenet for at holde op med at følge et produkt."
            },
            "sites": sites_field(c),
            "foreign": foreign_field(c),
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
    if let Some(f) = input.get("foreign").filter(|v| v.is_boolean()) {
        schema["properties"]["foreign"]["default"] = f.clone();
    }
    schema
}

/// The price alert step, with the countries and products kept in the flow.
fn alerts_form(flow: &Flow, c: &Choice, list: &[Product], current: &Alerts, errors: Option<Value>, input: Option<&Value>) -> FlowReply {
    let mut schema = alerts::schema(list, &c.sites, c.home, current);
    if let Some(i) = input {
        schema = alerts::keep_input(schema, i);
    }
    let mut data = flow.data.clone();
    c.insert(&mut data);
    data.insert("products".into(), products_json(list));
    form_with("alerts", schema, None, errors).with_data(data)
}

/// The alert step: the settings, or the form again.
fn alerts_step(flow: &Flow, current: &Alerts, input: Option<Value>) -> Result<(Choice, Vec<Product>, Alerts), Box<FlowReply>> {
    let c = Choice::from_flow(&flow.data);
    let list = product_list(flow.data.get("products"));
    match input {
        Some(v) => match alerts::from_input(&list, &c.sites, c.home, &v) {
            Ok(a) => Ok((c, list, a)),
            Err(e) => Err(Box::new(alerts_form(flow, &c, &list, current, Some(e), Some(&v)))),
        },
        None => Err(Box::new(alerts_form(flow, &c, &list, current, None, None))),
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
        api::parse_initial(&v).map(|p| p.from(site)).ok_or(Fetch::Missing)
    }

    async fn prices(&self, client: &reqwest::Client, site: &Site, p: &Product, foreign: bool) -> Result<Prices, Fetch> {
        let v = self.get(client, site, site.offers_path(&p.id, foreign)).await?;
        Ok(api::parse_offers(&v, site, foreign))
    }

    async fn search(&self, client: &reqwest::Client, site: &Site, q: &str) -> Result<Vec<Found>, Fetch> {
        let v = self.get(client, site, site.search_path(q)).await?;
        Ok(api::parse_search(&v)
            .into_iter()
            .take(FOUND)
            .map(|mut f| {
                f.product = f.product.from(site);
                f
            })
            .collect())
    }

    /// The products behind the pasted links, or the error code for the field.
    /// A link is looked up on its own site, a bare id on the first country.
    async fn resolve(&self, c: &Choice, links: &str) -> Result<Vec<Product>, Value> {
        let mut out = vec![];
        let client = client().map_err(|_| json!({ "base": "cannot_connect" }))?;
        for l in api::split_links(links) {
            let link = match api::parse_link(l) {
                Ok(link) => link,
                Err(LinkError::Bad) => return Err(json!({ "links": "bad_link" })),
                Err(LinkError::OtherSite) => return Err(json!({ "links": "other_site" })),
            };
            let site = match link.site {
                Some(s) if c.sites.contains(&s) => s,
                Some(_) => return Err(json!({ "links": "other_site" })),
                None => c.main(),
            };
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
    /// the options flow, with the entry's countries and products.
    async fn pick(&self, flow: &Flow, step: &str, input: Option<Value>, current: Option<(&Choice, &[Product])>) -> Result<Result<(Choice, Vec<Product>), FlowReply>, Error> {
        let first = if current.is_some() { "init" } else { "user" };
        let blank = |c: &Choice, keep: &[String]| match current {
            Some((_, list)) => init_schema(list, keep, c),
            None => user_schema(c),
        };
        match (step, input) {
            ("pick", Some(v)) => {
                let c = Choice::from_flow(&flow.data);
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
                Ok(Ok((c, list)))
            }
            (_, Some(v)) => {
                // `site` is the field from before there could be several countries
                let sites = match v.get("sites").filter(|s| !s.is_null()).or_else(|| v.get("site").filter(|s| !s.is_null())) {
                    Some(s) => sites_of(s),
                    None => current.map(|(c, _)| c.sites.clone()).unwrap_or_default(),
                };
                let foreign = v.get("foreign").and_then(Value::as_bool).unwrap_or(current.is_some_and(|(c, _)| c.foreign));
                let home = match current {
                    Some((c, _)) => c.home,
                    None => sites.first().copied().unwrap_or(&api::SITES[0]),
                };
                let c = Choice { sites, home, foreign };
                // a form sent without the field keeps every product
                let keep_ids = match v.get("keep") {
                    Some(k) if !k.is_null() => str_list(k),
                    _ => current.map(|(_, list)| list.iter().map(|p| p.id.clone()).collect()).unwrap_or_default(),
                };
                let again = |errors: Value| {
                    let schema = keep_input(blank(&c, &keep_ids), &v);
                    Ok(Err(form_with(first, schema, None, Some(errors))))
                };
                if c.sites.is_empty() {
                    return again(json!({ "sites": "no_sites" }));
                }
                let mut list: Vec<Product> = current.map(|(_, list)| list.iter().filter(|p| keep_ids.contains(&p.id)).cloned().collect()).unwrap_or_default();
                match self.resolve(&c, text(&v, "links")).await {
                    Ok(more) => add(&mut list, more),
                    Err(e) => return again(e),
                }
                let q = text(&v, "search");
                if !q.is_empty() {
                    let found = match self.search(&client()?, c.main(), q).await {
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
                    c.insert(&mut data);
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
                Ok(Ok((c, list)))
            }
            (_, None) => {
                let keep: Vec<String> = current.map(|(_, list)| list.iter().map(|p| p.id.clone()).collect()).unwrap_or_default();
                let c = current.map(|(c, _)| c.clone()).unwrap_or_default();
                Ok(Err(form_with(first, blank(&c, &keep), None, None)))
            }
        }
    }
}

#[async_trait]
impl Integration for Pricerunner {
    async fn config_flow(&self, _host: &Host, flow: Flow, step: &str, input: Option<Value>) -> Result<FlowReply, Error> {
        if step == "alerts" {
            return Ok(match alerts_step(&flow, &Alerts::default(), input) {
                Err(form) => *form,
                Ok((c, list, a)) => {
                    let title = format!("Pricerunner {}", c.sites.iter().map(|s| s.label()).collect::<Vec<_>>().join(", "));
                    let data = json!({ "site": c.home.key, "sites": c.keys(), "foreign": c.foreign, "products": products_json(&list) });
                    FlowReply::create(&title, with_alerts(data, &a))
                }
            });
        }
        match self.pick(&flow, step, input, None).await? {
            Err(form) => Ok(form),
            Ok((c, list)) => Ok(alerts_form(&flow, &c, &list, &Alerts::default(), None, None)),
        }
    }

    async fn options_flow(&self, _host: &Host, flow: Flow, step: &str, input: Option<Value>) -> Result<FlowReply, Error> {
        let Some(entry) = flow.entry.clone() else { return Ok(FlowReply::abort("unknown_entry")) };
        let current = product_list(entry.get("products"));
        let choice = Choice::from_entry(&entry);
        let set = Alerts::from_entry(&entry);
        if step == "alerts" {
            return Ok(match alerts_step(&flow, &set, input) {
                Err(form) => *form,
                Ok((c, list, a)) => FlowReply::create("", with_alerts(json!({ "sites": c.keys(), "foreign": c.foreign, "products": products_json(&list) }), &a)),
            });
        }
        match self.pick(&flow, step, input, Some((&choice, &current))).await? {
            Err(form) => Ok(form),
            Ok((c, list)) => Ok(alerts_form(&flow, &c, &list, &set, None, None)),
        }
    }

    async fn setup_entry(&self, host: &Host, entry: Entry) -> Result<(), Error> {
        let entry_id = entry.entry_id.clone();
        let choice = Choice::from_entry(&entry);
        // the products from before several countries have their name and path from the first one
        let products: Vec<Product> = product_list(entry.get("products")).into_iter().map(|p| if p.site.is_none() { p.from(choice.home) } else { p }).collect();
        if products.is_empty() {
            return Err(Error::failed("ingen produkter"));
        }
        let several = choice.sites.len() > 1;
        let alerts = Alerts::from_entry(&entry).keep(&alerts::slots(&products, &choice.sites, choice.home));
        let sent = Sent::new(&host.data_dir());
        sent.retain(&entry_id, &alerts);
        let client = client()?;
        let pairs: Vec<(&Product, &'static Site)> = products.iter().flat_map(|p| choice.sites.iter().map(move |s| (p, *s))).collect();
        // the first fetch here, so a setup without a connection is retried
        let mut first = vec![];
        for (i, (p, site)) in pairs.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(self.gap).await;
            }
            first.push(self.prices(&client, site, p, choice.foreign).await);
        }
        if first.iter().all(|r| matches!(r, Err(Fetch::Failed(_)))) {
            if let Some(Err(e)) = first.first() {
                return Err(Error::not_ready(e));
            }
        }

        let mut watched = vec![];
        for product in &products {
            let device = json!({
                "identifiers": [[DOMAIN, format!("{entry_id}-{}", product.id)]],
                "name": product.name,
                "manufacturer": product.brand.clone().unwrap_or_else(|| "Pricerunner".into()),
                "model": product.category_name.clone().unwrap_or_else(|| "Produkt".into()),
                "entry_type": "service",
                "configuration_url": choice.main().product_url(product),
            });
            let dev = host.request("device/register", json!({ "entry_id": entry_id, "device": device })).await?;
            let dev_id = dev["id"].as_str().ok_or_else(|| Error::failed("intet enheds-id"))?.to_string();
            for &site in &choice.sites {
                let slot = alerts::slot(&product.id, site, choice.home);
                let uid = format!("{entry_id}-{}", slot.replace('@', "-"));
                let name = |n: &str| if several { format!("{n} {}", site.land) } else { n.to_string() };
                let price = Entity::new("sensor", format!("{uid}-price"))
                    .name(&name("Laveste pris"))
                    .device_class("monetary")
                    .unit(site.currency)
                    .state_class("measurement")
                    .icon("mdi:tag-outline");
                let price = host.add_entity(&entry_id, price.device(&dev_id), false).await?;
                let shop = Entity::new("sensor", format!("{uid}-shop")).name(&name("Billigste butik")).icon("mdi:storefront-outline");
                let shop = host.add_entity(&entry_id, shop.device(&dev_id), false).await?;
                let url = site.product_url(product);
                watched.push(Watched { product: product.clone(), site, slot, land: several.then_some(site.land), url, price, shop, failures: 0 });
            }
        }
        for (w, r) in watched.iter_mut().zip(first) {
            match r {
                Ok(p) => {
                    w.show(host, &p).await;
                    alerts::check(host, &sent, &alerts, &entry_id, &w.watch(), p.cheapest()).await;
                }
                Err(e) => {
                    warn!("{} ({}): {e}", w.product.name, w.site.label());
                    w.failures = 1;
                }
            }
        }

        let host = host.clone();
        let this = self.copy();
        let id = entry_id.clone();
        let foreign = choice.foreign;
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(this.poll).await;
                for (i, w) in watched.iter_mut().enumerate() {
                    if i > 0 {
                        tokio::time::sleep(this.gap).await;
                    }
                    match this.prices(&client, w.site, &w.product, foreign).await {
                        Ok(p) => {
                            w.failures = 0;
                            w.show(&host, &p).await;
                            alerts::check(&host, &sent, &alerts, &id, &w.watch(), p.cheapest()).await;
                        }
                        Err(e) => {
                            w.failures += 1;
                            warn!("{} ({}): {e}", w.product.name, w.site.label());
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

/// A product in one country, and its two entities.
struct Watched {
    product: Product,
    site: &'static Site,
    /// The key of its price alert.
    slot: String,
    /// The country, when the entry has several.
    land: Option<&'static str>,
    url: String,
    price: Option<String>,
    shop: Option<String>,
    failures: u32,
}

fn offer_json(o: &api::Offer) -> Value {
    json!({ "shop": o.shop, "price": o.price, "shipping": o.shipping, "stock": o.stock.to_lowercase(), "url": o.url })
}

impl Watched {
    fn watch(&self) -> Watch<'_> {
        Watch { product: &self.product, slot: &self.slot, land: self.land, url: &self.url }
    }

    async fn show(&self, host: &Host, prices: &Prices) {
        let site = self.site;
        let cheapest = prices.cheapest();
        let attribution = json!(if site.is_klarna() { "Priser fra Klarna" } else { "Priser fra Pricerunner" });
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
            a.insert("country".into(), json!(site.key));
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
            a.insert("country".into(), json!(site.key));
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
